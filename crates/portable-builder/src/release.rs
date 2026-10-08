//! Port of portable_builder/release.py - release rendering + GitHub API. Owner: Wave3-B.
//!
//! Semantics follow the Python source exactly (release_check_contract.md and
//! its addendum are the contract of record):
//! - check decision tree produces the UPDATE_NEEDED / UPSTREAM_VERSION /
//!   CREATE_NEW_RELEASE / MINOR_UPDATE (+ conditional RELEASE_ID / RELEASE_TAG)
//!   env blob via github_env::write_env;
//! - asset matchers chain release.asset_regex > archive_name (compiled from
//!   the template with PLACEHOLDER_PATTERNS) > asset_match (contains/exact/
//!   prefix/suffix/regex) > default output_dir contains;
//! - GitHub API: 204-only delete, browser_download_url first with an
//!   application/octet-stream fallback header for API URLs, substring
//!   case-insensitive pattern sweep;
//! - update-release carries body only; name+tag_name only on MINOR_UPDATE=true
//!   (the in-place version-advance signal).
//!
//! Asset UPLOAD is deliberately NOT implemented: the workflow's
//! softprops/action-gh-release step uploads build/assets/*.7z (contract
//! boundary, multi_contract.md addendum 2 note).
//!
//! Porting notes live in docs/MIGRATION_RUST_TAURI.md section 4.6.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use regex::RegexBuilder;
use serde_json::Value;

use crate::github_env::{build_run_url, write_env};
use crate::log_fmt::info;
use crate::tools::human_size;
use crate::versions::{is_upgrade, should_create_new_release};

/// release.py DEFAULT_SAMPLE_VERSION - the dry-run sample version fed into
/// matcher-overlap checks and archive-name regexes.
pub const DEFAULT_SAMPLE_VERSION: &str = "123.456.789.0";
/// release.py DEFAULT_SAMPLE_DATE.
pub const DEFAULT_SAMPLE_DATE: &str = "2099-12-31";

/// release.py PLACEHOLDER_PATTERNS: the regex fragment substituted for each
/// template placeholder when compiling archive_name into a matcher.
pub const PLACEHOLDER_PATTERNS: [(&str, &str); 4] = [
    ("version", r"\d+(?:\.\d+)+"),
    ("package_version", r"\d+(?:\.\d+)+"),
    ("date", r"\d{4}-\d{2}-\d{2}"),
    ("arch", r"[A-Za-z0-9][A-Za-z0-9.-]*"),
];

/// One compiled asset matcher: the description string (the diagnostics half
/// of Python's (description, matcher) tuples) plus the predicate.
pub struct AssetMatcher {
    pub description: String,
    pub predicate: Box<dyn Fn(&str) -> bool>,
}

fn target_str<'a>(target: &'a Value, key: &str) -> Option<&'a str> {
    target.get(key).and_then(Value::as_str)
}

/// Python target.get("name", target.get("display_name", "")).
fn name_of(target: &Value) -> String {
    target_str(target, "name")
        .or_else(|| target_str(target, "display_name"))
        .unwrap_or("")
        .to_string()
}

/// Python target.get("display_name", target.get("name", "")).
fn display_name_of(target: &Value) -> String {
    target_str(target, "display_name")
        .or_else(|| target_str(target, "name"))
        .unwrap_or("")
        .to_string()
}

/// Python target.get("output_dir", target.get("name", "Browser")).
fn output_dir_of(target: &Value) -> String {
    target_str(target, "output_dir")
        .or_else(|| target_str(target, "name"))
        .unwrap_or("Browser")
        .to_string()
}

/// Python target.get("release", {}) - a missing release block reads as an
/// empty mapping.
pub fn target_release_config(target: &Value) -> Value {
    target
        .get("release")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}))
}

/// Port of builder.py::format_value over a str.format context map: bare
/// {name} fields only (the frozen crate formatter subset, carry-forward
/// item 1). A missing key raises like Python's KeyError.
pub fn format_value(template: &str, context: &[(String, String)]) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if bytes.get(i + 1) == Some(&b'{') => {
                out.push('{');
                i += 2;
            }
            b'}' if bytes.get(i + 1) == Some(&b'}') => {
                out.push('}');
                i += 2;
            }
            b'{' => {
                let end = i
                    + 1
                    + bytes[i + 1..]
                        .iter()
                        .position(|&b| b == b'}')
                        .ok_or_else(|| anyhow!("Single '{{' encountered in format string"))?;
                let field = &template[i + 1..end];
                let value = context
                    .iter()
                    .find(|(key, _)| key == field)
                    .map(|(_, value)| value.as_str())
                    .ok_or_else(|| anyhow!("KeyError: '{field}'"))?;
                out.push_str(value);
                i = end + 1;
            }
            b'}' => bail!("Single '}}' encountered in format string"),
            _ => {
                let ch = template[i..].chars().next().expect("char boundary");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    Ok(out)
}

/// Port of builder.py::build_context (the release-facing subset): the
/// version / date / package_version fallbacks happen at the callers; here
/// they are already resolved. The archive-stage keys read ARCHIVE_NAME /
/// ARCHIVE_SHA256 / ARCHIVE_SIZE (empty when rendering before archiving)
/// plus CHROME_PLUS_VERSION and run_url.
pub fn build_context(
    target: &Value,
    version: &str,
    date: &str,
    package_version: &str,
) -> Vec<(String, String)> {
    let env = |name: &str| std::env::var(name).unwrap_or_default();
    let raw_size = env("ARCHIVE_SIZE");
    // Python: human_size(os.getenv("ARCHIVE_SIZE") or None) - a falsy env
    // (unset or empty) maps to None, anything else parses as a float.
    let size: Option<f64> = if raw_size.is_empty() {
        None
    } else {
        raw_size.parse::<f64>().ok()
    };
    vec![
        (
            "target".to_string(),
            target_str(target, "target").unwrap_or("").to_string(),
        ),
        ("name".to_string(), name_of(target)),
        ("display_name".to_string(), display_name_of(target)),
        ("output_dir".to_string(), output_dir_of(target)),
        ("version".to_string(), version.to_string()),
        ("package_version".to_string(), package_version.to_string()),
        ("date".to_string(), date.to_string()),
        (
            "arch".to_string(),
            target_str(target, "architecture")
                .unwrap_or("x64")
                .to_string(),
        ),
        ("archive".to_string(), env("ARCHIVE_NAME")),
        ("sha256".to_string(), env("ARCHIVE_SHA256")),
        ("size".to_string(), human_size(size)),
        (
            "chrome_plus_version".to_string(),
            env("CHROME_PLUS_VERSION"),
        ),
        ("run_url".to_string(), build_run_url()),
    ]
}

/// Port of release.py::github_headers - token optional, an empty GITHUB_TOKEN
/// is falsy in Python so no Authorization header is emitted.
pub fn github_headers() -> Vec<(String, String)> {
    let mut headers = vec![(
        "Accept".to_string(),
        "application/vnd.github.v3+json".to_string(),
    )];
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            headers.push(("Authorization".to_string(), format!("token {token}")));
        }
    }
    headers
}

fn api_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?)
}

fn apply_headers(
    request: reqwest::blocking::RequestBuilder,
    headers: &[(String, String)],
) -> reqwest::blocking::RequestBuilder {
    let mut request = request;
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request
}

/// Port of release.py::latest_release. A missing/empty GITHUB_REPOSITORY
/// prints the first-local-run notice and returns None; 404 maps to None (no
/// release yet); anything else non-OK raises like raise_for_status.
pub fn latest_release() -> Result<Option<Value>> {
    let repo = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
    if repo.is_empty() {
        info("GITHUB_REPOSITORY is not set; assuming first local run.");
        return Ok(None);
    }

    let client = api_client()?;
    let response = apply_headers(
        client.get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        )),
        &github_headers(),
    )
    .send()?;
    if response.status().as_u16() == 404 {
        return Ok(None);
    }
    let response = response.error_for_status()?;
    Ok(Some(response.json()?))
}

/// Port of release.py::extract_version - first d+.d+.d+.d+ run, found by the
/// same greedy left-to-right scan Python re.search performs.
pub fn extract_version(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        // A candidate match starts here: four dot-joined digit runs.
        let mut cursor = index;
        let mut groups = 0;
        let mut end;
        loop {
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            groups += 1;
            end = cursor;
            // Greedy: a dot followed by at least one digit continues the match.
            if cursor < bytes.len()
                && bytes[cursor] == b'.'
                && cursor + 1 < bytes.len()
                && bytes[cursor + 1].is_ascii_digit()
            {
                cursor += 1;
                continue;
            }
            break;
        }
        if groups >= 4 {
            return Some(text[index..end].to_string());
        }
        // Not a 4-part run: retry from the next character (re.search semantics).
        index += 1;
    }
    None
}
/// Port of release.py::asset_context with fill_defaults=True.
fn asset_context_filled(
    target: &Value,
    version: &str,
    date: &str,
    package_version: Option<&str>,
) -> Vec<(String, String)> {
    let package_version = package_version.unwrap_or(version);
    build_context(target, version, date, package_version)
}

/// Port of release.py::asset_context with fill_defaults=False: the bare
/// skeleton used to compile archive_name templates into matchers.
fn asset_context_skeleton(target: &Value) -> Vec<(String, String)> {
    vec![
        (
            "target".to_string(),
            target_str(target, "target").unwrap_or("").to_string(),
        ),
        ("name".to_string(), name_of(target)),
        ("display_name".to_string(), display_name_of(target)),
        ("output_dir".to_string(), output_dir_of(target)),
        ("version".to_string(), String::new()),
        ("package_version".to_string(), String::new()),
        ("date".to_string(), String::new()),
        (
            "arch".to_string(),
            target_str(target, "architecture")
                .unwrap_or("x64")
                .to_string(),
        ),
    ]
}

/// Port of release.py::asset_name_template.
pub fn asset_name_template(target: &Value) -> String {
    target_str(target, "archive_name")
        .unwrap_or("{display_name}_Portable_{version}_{date}.7z")
        .to_string()
}

/// Port of release.py::render_asset_name.
pub fn render_asset_name(
    target: &Value,
    version: &str,
    date: &str,
    package_version: Option<&str>,
) -> Result<String> {
    let context = asset_context_filled(target, version, date, package_version);
    format_value(&asset_name_template(target), &context)
}

/// Python re.escape (3.12): keeps [a-zA-Z0-9_] verbatim, escapes every other
/// ASCII character with a backslash. Non-ASCII passes through.
fn regex_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('\\');
            out.push(ch);
        }
    }
    out
}

/// Python re.IGNORECASE compile.
fn compile_pattern(pattern: &str) -> Result<regex::Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map_err(|exc| anyhow!("invalid matcher regex {pattern:?}: {exc}"))
}

/// Port of release.py::archive_name_regex, returned as pattern text.
pub fn archive_name_regex_text(target: &Value) -> Result<String> {
    let template = asset_name_template(target);
    let context = asset_context_skeleton(target);
    let mut parts = vec!["^".to_string()];
    // Walk the template like string.Formatter().parse: literal text, then
    // {field} placeholders ({{ }} escapes first).
    let bytes = template.as_bytes();
    let mut literal = String::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if bytes.get(i + 1) == Some(&b'{') => {
                literal.push('{');
                i += 2;
            }
            b'}' if bytes.get(i + 1) == Some(&b'}') => {
                literal.push('}');
                i += 2;
            }
            b'{' => {
                let end = i
                    + 1
                    + bytes[i + 1..]
                        .iter()
                        .position(|&b| b == b'}')
                        .ok_or_else(|| anyhow!("Single '{{' encountered in format string"))?;
                let field = &template[i + 1..end];
                parts.push(regex_escape(&literal));
                literal.clear();
                // Python: value = context.get(field); value not in (None, "")
                // -> escape it verbatim, else the placeholder pattern.
                let resolved = context
                    .iter()
                    .find(|(key, _)| key == field)
                    .map(|(_, value)| value.as_str())
                    .filter(|value| !value.is_empty());
                match resolved {
                    Some(value) => parts.push(regex_escape(value)),
                    None => parts.push(
                        PLACEHOLDER_PATTERNS
                            .iter()
                            .find(|(name, _)| *name == field)
                            .map(|(_, pattern)| pattern.to_string())
                            .unwrap_or_else(|| r"[^/]+?".to_string()),
                    ),
                }
                i = end + 1;
            }
            b'}' => bail!("Single '}}' encountered in format string"),
            _ => {
                let ch = template[i..].chars().next().expect("char boundary");
                literal.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    parts.push(regex_escape(&literal));
    parts.push("$".to_string());
    Ok(parts.concat())
}

/// Python str(asset_match) over a JSON value: strings stay verbatim; bools
/// render as Python True/False. Real configs only carry strings.
fn asset_match_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

/// Port of release.py::asset_matchers - the ordered matcher chain.
pub fn asset_matchers(target: &Value) -> Result<Vec<AssetMatcher>> {
    let release = target_release_config(target);
    let mut matchers: Vec<AssetMatcher> = Vec::new();

    let asset_regex = release
        .get("asset_regex")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !asset_regex.is_empty() {
        let compiled = compile_pattern(asset_regex)?;
        matchers.push(AssetMatcher {
            description: "release.asset_regex".to_string(),
            predicate: Box::new(move |name: &str| compiled.is_match(name)),
        });
    }

    if target.get("archive_name").and_then(Value::as_str).is_some() {
        let pattern = archive_name_regex_text(target)?;
        let anchored = compile_pattern(&format!("^{pattern}$"))?;
        let description = format!("archive_name:{}", asset_name_template(target));
        // archive_name uses re.fullmatch semantics: the compiled pattern is
        // anchored on both ends (Python regex.fullmatch(name)).
        matchers.push(AssetMatcher {
            description,
            predicate: Box::new(move |name: &str| anchored.is_match(name)),
        });
    }

    let asset_match = release.get("asset_match").map(asset_match_string);
    if let Some(asset_match) = asset_match {
        if !asset_match.is_empty() {
            let mode = release
                .get("asset_match_mode")
                .and_then(Value::as_str)
                .unwrap_or("contains")
                .to_lowercase();
            let description;
            let predicate: Box<dyn Fn(&str) -> bool>;
            match mode.as_str() {
                "exact" => {
                    description = format!("asset_match(exact):{asset_match}");
                    let needle = asset_match.to_lowercase();
                    predicate = Box::new(move |name: &str| name.to_lowercase() == needle);
                }
                "prefix" => {
                    description = format!("asset_match(prefix):{asset_match}");
                    let needle = asset_match.to_lowercase();
                    predicate =
                        Box::new(move |name: &str| name.to_lowercase().starts_with(&needle));
                }
                "suffix" => {
                    description = format!("asset_match(suffix):{asset_match}");
                    let needle = asset_match.to_lowercase();
                    predicate = Box::new(move |name: &str| name.to_lowercase().ends_with(&needle));
                }
                "regex" => {
                    description = format!("asset_match(regex):{asset_match}");
                    let compiled = compile_pattern(&asset_match)?;
                    predicate = Box::new(move |name: &str| compiled.is_match(name));
                }
                _ => {
                    description = format!("asset_match(contains):{asset_match}");
                    let needle = asset_match.to_lowercase();
                    predicate = Box::new(move |name: &str| name.to_lowercase().contains(&needle));
                }
            }
            matchers.push(AssetMatcher {
                description,
                predicate,
            });
        }
    }

    if matchers.is_empty() {
        let fallback = output_dir_of(target);
        if !fallback.is_empty() {
            let description = format!("default(contains):{fallback}");
            let needle = fallback.to_lowercase();
            matchers.push(AssetMatcher {
                description,
                predicate: Box::new(move |name: &str| name.to_lowercase().contains(&needle)),
            });
        }
    }

    Ok(matchers)
}

/// Port of release.py::matching_assets_for_target - the first matcher with
/// any hit wins; (matches, Some(description)) or ([], None).
pub fn matching_assets_for_target<'a>(
    assets: &'a [Value],
    target: &Value,
) -> Result<(Vec<&'a Value>, Option<String>)> {
    for matcher in asset_matchers(target)? {
        let matches: Vec<&Value> = assets
            .iter()
            .filter(|asset| {
                (matcher.predicate)(asset.get("name").and_then(Value::as_str).unwrap_or(""))
            })
            .collect();
        if !matches.is_empty() {
            return Ok((matches, Some(matcher.description)));
        }
    }
    Ok((Vec::new(), None))
}

/// Port of release.py::asset_matches_target.
pub fn asset_matches_target(target: &Value, asset_name: &str) -> Result<bool> {
    Ok(asset_matchers(target)?
        .iter()
        .any(|m| (m.predicate)(asset_name)))
}

/// Port of release.py::target_match_description - ", "-joined descriptions or
/// "(none)" for an empty chain.
pub fn target_match_description(target: &Value) -> Result<String> {
    let descriptions: Vec<String> = asset_matchers(target)?
        .into_iter()
        .map(|m| m.description)
        .collect();
    Ok(if descriptions.is_empty() {
        "(none)".to_string()
    } else {
        descriptions.join(", ")
    })
}

/// Port of release.py::release_version - version_pattern against the body
/// first (group 1), then extract_version over the tag, then over the body.
pub fn release_version(release: Option<&Value>, target: &Value) -> Option<String> {
    let release = release?;
    let release_config = target_release_config(target);
    let body = release.get("body").and_then(Value::as_str).unwrap_or("");
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    if let Some(pattern) = release_config
        .get("version_pattern")
        .and_then(Value::as_str)
    {
        if !pattern.is_empty() {
            if let Ok(compiled) = compile_pattern(pattern) {
                if let Some(group) = compiled.captures(body).and_then(|c| c.get(1)) {
                    return Some(group.as_str().to_string());
                }
            }
        }
    }
    extract_version(tag).or_else(|| extract_version(body))
}

/// Port of release.py::assert_body_versions. expectations is the ordered
/// list of (target_name, pattern, expected_version).
pub fn assert_body_versions(
    body: &str,
    expectations: &[(String, Option<String>, Option<String>)],
) -> Result<()> {
    let mut problems: Vec<String> = Vec::new();
    for (target_name, pattern, expected) in expectations {
        let (Some(pattern), Some(expected)) = (pattern, expected) else {
            continue;
        };
        if pattern.is_empty() || expected.is_empty() {
            continue;
        }
        let compiled = compile_pattern(pattern)?;
        match compiled.captures(body).and_then(|c| c.get(1)) {
            None => problems.push(format!(
                "{target_name}: pattern {pattern:?} matches nothing in the rendered body"
            )),
            Some(group) => {
                if group.as_str() != expected {
                    problems.push(format!(
                        "{target_name}: pattern {pattern:?} read '{}', expected '{expected}'",
                        group.as_str()
                    ));
                }
            }
        }
    }

    if !problems.is_empty() {
        bail!(
            "Release body and version_pattern disagree; the check step would not find the \
             published version and every scheduled run would rebuild. {}",
            problems.join("; ")
        );
    }

    let checked: Vec<&String> = expectations
        .iter()
        .filter(|(_, pattern, expected)| {
            pattern.as_deref().is_some_and(|p| !p.is_empty())
                && expected.as_deref().is_some_and(|e| !e.is_empty())
        })
        .map(|(name, _, _)| name)
        .collect();
    if !checked.is_empty() {
        info(format!(
            "version_pattern round-trip verified for: {}",
            checked
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

/// Python str(bool): "true" / "false".
fn bool_py(value: bool) -> String {
    if value {
        "true".to_string()
    } else {
        "false".to_string()
    }
}

/// Port of builder.py::get_version_info - provider config copy + _workdir,
/// dispatched over the four provider modules (Value-based API).
pub fn get_version_info(target: &Value, workdir: Option<&Path>) -> Result<Value> {
    let mut provider_config = target
        .get("provider")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(workdir) = workdir {
        if let Some(map) = provider_config.as_object_mut() {
            map.insert(
                "_workdir".to_string(),
                Value::String(workdir.to_string_lossy().into_owned()),
            );
        }
    }
    crate::providers::get_package(&provider_config)
}

/// The frozen five-branch decision tree, factored out for the golden replay
/// (release_check_contract.md). Pure over its inputs; the caller does the
/// printing so the log lines stay exactly where Python's if/elif chain emits
/// them.
pub fn decide_update_needed(
    force_build: bool,
    current_version: Option<&str>,
    asset_present: bool,
    upstream_version: &str,
) -> (bool, &'static str) {
    if force_build {
        (true, "Manual dispatch detected; forcing build.")
    } else if current_version.is_none() {
        (true, "No existing release version found; build is needed.")
    } else if !asset_present {
        (true, "Release asset is missing; rebuild is needed.")
    } else if upstream_version != current_version.unwrap_or("")
        && is_upgrade(upstream_version, current_version.unwrap_or(""))
    {
        (true, "Version upgrade detected")
    } else {
        (false, "No newer upstream version detected.")
    }
}

/// Port of release.py::check_updates - the frozen five-branch decision tree
/// plus the env blob (UPDATE_NEEDED / UPSTREAM_VERSION / CREATE_NEW_RELEASE /
/// MINOR_UPDATE, with RELEASE_ID / RELEASE_TAG appended when present).
pub fn check_updates(target: &Value, workdir: &Path) -> Result<Vec<(String, String)>> {
    let event_name = std::env::var("GITHUB_EVENT_NAME").unwrap_or_default();
    let force_build = event_name == "workflow_dispatch";

    let package = get_version_info(target, Some(workdir))?;
    let upstream_version = package
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let release = latest_release()?;
    let current_version = release
        .as_ref()
        .and_then(|r| release_version(Some(r), target));
    let release_id = release
        .as_ref()
        .and_then(|r| r.get("id"))
        .and_then(Value::as_i64);
    let release_tag = release
        .as_ref()
        .and_then(|r| r.get("tag_name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let asset_present = match release_id {
        Some(id) => !find_matching_release_assets(id, target)?.is_empty(),
        None => false,
    };

    info(format!("Upstream version: {upstream_version}"));
    info(format!(
        "Current release version: {}",
        current_version.as_deref().unwrap_or("None")
    ));
    info(format!("Matching release asset present: {asset_present}"));

    // The decision tree, branch by branch (release_check_contract.md);
    // decide_update_needed holds the branch order, the prints stay here so
    // the log lines land exactly where Python's chain emits them.
    let (update_needed, branch_message) = decide_update_needed(
        force_build,
        current_version.as_deref(),
        asset_present,
        &upstream_version,
    );
    if branch_message == "Version upgrade detected" {
        info(format!(
            "Version upgrade detected: {} -> {}",
            current_version.as_deref().unwrap_or(""),
            upstream_version
        ));
    } else {
        info(branch_message);
    }

    let create_policy = target_release_config(target)
        .get("create_new_release_on")
        .and_then(Value::as_str)
        .map(str::to_string);
    let create_new_release = release_id.is_none()
        || (update_needed
            && current_version.is_some()
            && should_create_new_release(
                create_policy.as_deref(),
                &upstream_version,
                current_version.as_deref().unwrap_or(""),
            )?);
    // In-place rename only when we stay on the existing release.
    let minor_update = update_needed
        && current_version.is_some()
        && !create_new_release
        && upstream_version != current_version.clone().unwrap_or_default()
        && is_upgrade(&upstream_version, current_version.as_deref().unwrap_or(""));

    let mut values: Vec<(String, String)> = vec![
        ("UPDATE_NEEDED".to_string(), bool_py(update_needed)),
        ("UPSTREAM_VERSION".to_string(), upstream_version),
        (
            "CREATE_NEW_RELEASE".to_string(),
            bool_py(create_new_release),
        ),
        ("MINOR_UPDATE".to_string(), bool_py(minor_update)),
    ];
    if let Some(id) = release_id {
        values.push(("RELEASE_ID".to_string(), id.to_string()));
    }
    if let Some(tag) = release_tag {
        if !tag.is_empty() {
            values.push(("RELEASE_TAG".to_string(), tag));
        }
    }
    write_env(&values)?;
    Ok(values)
}

/// Port of release.py::render_release (single target). Version fallback
/// chain: argument > BUILT_VERSION > BROWSER_VERSION > UPSTREAM_VERSION;
/// date chain: argument > BUILD_DATE > today. Body lands in
/// build/release_body.md (UTF-8, no BOM) and the env carries RELEASE_TAG /
/// RELEASE_TITLE / RELEASE_BODY_PATH.
pub fn render_release(
    target: &Value,
    workdir: &Path,
    version: Option<&str>,
    build_date: Option<&str>,
) -> Result<Value> {
    let env = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let version = version
        .map(str::to_string)
        .or_else(|| env("BUILT_VERSION"))
        .or_else(|| env("BROWSER_VERSION"))
        .or_else(|| env("UPSTREAM_VERSION"))
        .unwrap_or_default();
    let build_date = build_date
        .map(str::to_string)
        .or_else(|| env("BUILD_DATE"))
        .unwrap_or_else(today);

    let context = build_context(target, &version, &build_date, &version);
    let release = target_release_config(target);

    let tag_template = release
        .get("tag")
        .and_then(Value::as_str)
        .unwrap_or("v{version}");
    let title_template = release
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("{display_name} {version}");
    let body_template = release.get("body").and_then(Value::as_str).unwrap_or(
        "\u{81ea}\u{52a8}\u{6784}\u{5efa}\u{7684} {display_name} \u{4fbf}\u{643a}\u{7248}\n\n\u{6784}\u{5efa}\u{65f6}\u{95f4}: {date}\n{display_name} \u{7248}\u{672c}: {version}\n",
    );

    let tag = format_value(tag_template, &context)?;
    let title = format_value(title_template, &context)?;
    let body = format_value(body_template, &context)?;

    let target_name = target_str(target, "target").unwrap_or("target").to_string();
    let pattern = release
        .get("version_pattern")
        .and_then(Value::as_str)
        .map(str::to_string);
    let expected = if version.is_empty() {
        None
    } else {
        Some(version.clone())
    };
    assert_body_versions(&body, &[(target_name, pattern, expected)])?;

    let body_path = workdir.join("build").join("release_body.md");
    if let Some(parent) = body_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&body_path, &body)?;

    let values = vec![
        ("RELEASE_TAG".to_string(), tag.clone()),
        ("RELEASE_TITLE".to_string(), title.clone()),
        (
            "RELEASE_BODY_PATH".to_string(),
            body_path.to_string_lossy().into_owned(),
        ),
    ];
    write_env(&values)?;
    Ok(serde_json::json!({
        "tag": tag,
        "title": title,
        "body_path": body_path.to_string_lossy(),
    }))
}

/// Python datetime.now().strftime("%Y-%m-%d") via chrono.
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Port of release.py::delete_release_asset - 204 is the only success code.
pub fn delete_release_asset(asset_id: i64) -> Result<()> {
    let repo = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
    let client = api_client()?;
    let response = apply_headers(
        client.delete(format!(
            "https://api.github.com/repos/{repo}/releases/assets/{asset_id}"
        )),
        &github_headers(),
    )
    .send()?;
    if response.status().as_u16() != 204 {
        let status = response.status().as_u16();
        let text = response.text().unwrap_or_default();
        bail!("Failed to delete asset {asset_id}: {status} {text}");
    }
    Ok(())
}

/// Port of release.py::get_release_assets - raise_for_status then .assets.
pub fn get_release_assets(release_id: i64) -> Result<Vec<Value>> {
    let repo = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
    let client = api_client()?;
    let response = apply_headers(
        client.get(format!(
            "https://api.github.com/repos/{repo}/releases/{release_id}"
        )),
        &github_headers(),
    )
    .send()?
    .error_for_status()?;
    let payload: Value = response.json()?;
    Ok(payload
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// Port of release.py::find_matching_release_assets.
pub fn find_matching_release_assets(release_id: i64, target: &Value) -> Result<Vec<Value>> {
    let assets = get_release_assets(release_id)?;
    Ok(matching_assets_for_target(&assets, target)?
        .0
        .into_iter()
        .cloned()
        .collect())
}

/// Port of release.py::find_latest_target_asset - sort key chain
/// updated_at > updatedAt > created_at > createdAt > "" (reverse-sorted,
/// first wins; Python sorted(..., key=sort_key, reverse=True)[0]).
pub fn find_latest_target_asset(release_id: i64, target: &Value) -> Result<Option<Value>> {
    let mut matches = find_matching_release_assets(release_id, target)?;
    if matches.is_empty() {
        return Ok(None);
    }
    matches.sort_by_key(sort_key_rev);
    Ok(Some(matches.swap_remove(0)))
}

/// Python sort_key: the first non-empty timestamp field, else "".
fn sort_key_rev(asset: &Value) -> std::cmp::Reverse<String> {
    for key in ["updated_at", "updatedAt", "created_at", "createdAt"] {
        if let Some(text) = asset.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return std::cmp::Reverse(text.to_string());
            }
        }
    }
    std::cmp::Reverse(String::new())
}

/// Port of release.py::download_release_asset - browser_download_url first;
/// the API url path swaps the Accept header to application/octet-stream
/// (different auth behavior between the two URL kinds); 1 MiB streaming.
pub fn download_release_asset(asset: &Value, destination: &Path) -> Result<PathBuf> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let browser_url = asset
        .get("browser_download_url")
        .and_then(Value::as_str)
        .unwrap_or("");
    let api_url = asset.get("url").and_then(Value::as_str).unwrap_or("");
    let download_url = if !browser_url.is_empty() {
        browser_url
    } else {
        api_url
    };
    if download_url.is_empty() {
        bail!(
            "Release asset is missing a download URL: {}",
            asset
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("(unknown)")
        );
    }

    let mut headers = github_headers();
    if browser_url.is_empty() && !api_url.is_empty() {
        // Python builds a fresh dict and overwrites Accept.
        headers.retain(|(name, _)| name != "Accept");
        headers.push(("Accept".to_string(), "application/octet-stream".to_string()));
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let mut response = apply_headers(client.get(download_url), &headers)
        .send()?
        .error_for_status()?;
    let mut file = std::fs::File::create(destination)?;
    std::io::copy(&mut response, &mut file)?;

    Ok(destination.to_path_buf())
}

/// Port of release.py::delete_assets_by_pattern - substring,
/// case-insensitive (pattern.lower() in name.lower()).
pub fn delete_assets_by_pattern(release_id: i64, pattern: &str) -> Result<()> {
    let mut deleted = 0usize;
    for asset in get_release_assets(release_id)? {
        let name = asset.get("name").and_then(Value::as_str).unwrap_or("");
        if name.to_lowercase().contains(&pattern.to_lowercase()) {
            info(format!("Deleting old asset: {name}"));
            let id = asset.get("id").and_then(Value::as_i64).unwrap_or_default();
            delete_release_asset(id)?;
            deleted += 1;
        }
    }
    info(format!("Deleted {deleted} old assets."));
    Ok(())
}

/// Port of release.py::delete_target_assets - matcher-hit sweep plus the
/// diagnostic line when nothing matched (Python: description or
/// 'no matcher').
pub fn delete_target_assets(release_id: i64, target: &Value) -> Result<()> {
    let assets = get_release_assets(release_id)?;
    let (matches, description) = matching_assets_for_target(&assets, target)?;
    let mut deleted = 0usize;
    for asset in &matches {
        let name = asset.get("name").and_then(Value::as_str).unwrap_or("");
        info(format!("Deleting old asset: {name}"));
        let id = asset.get("id").and_then(Value::as_i64).unwrap_or_default();
        delete_release_asset(id)?;
        deleted += 1;
    }

    let target_label = target_str(target, "target")
        .or_else(|| target_str(target, "name"))
        .unwrap_or("target");
    if deleted == 0 {
        info(format!(
            "Deleted 0 old assets for {target_label} using {}.",
            description.unwrap_or_else(|| "no matcher".to_string())
        ));
    } else {
        info(format!("Deleted {deleted} old assets for {target_label}."));
    }
    Ok(())
}

/// Port of release.py::update_release. No RELEASE_ID prints the softprops
/// hand-off line and returns; the PATCH body carries body text only, with
/// name+tag_name added on MINOR_UPDATE=true.
pub fn update_release(target: &Value, workdir: &Path) -> Result<()> {
    let release_id = std::env::var("RELEASE_ID").unwrap_or_default();
    if release_id.is_empty() {
        info("No RELEASE_ID set; create release step will handle publishing.");
        return Ok(());
    }
    let release_id: i64 = release_id.parse()?;

    let release_info = render_release(target, workdir, None, None)?;
    delete_target_assets(release_id, target)?;

    let repo = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
    let body = std::fs::read_to_string(
        release_info
            .get("body_path")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    let mut data = serde_json::json!({ "body": body });
    if std::env::var("MINOR_UPDATE")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        == "true"
    {
        data["name"] = release_info.get("title").cloned().unwrap_or_default();
        data["tag_name"] = release_info.get("tag").cloned().unwrap_or_default();
    }

    let client = api_client()?;
    apply_headers(
        client.patch(format!(
            "https://api.github.com/repos/{repo}/releases/{release_id}"
        )),
        &github_headers(),
    )
    .json(&data)
    .send()?
    .error_for_status()?;
    info(format!("Updated release {release_id}."));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn probe_target() -> Value {
        json!({
            "target": "probe_stable",
            "name": "Probe",
            "display_name": "Probe Browser",
            "output_dir": "ProbeBrowser",
            "architecture": "x64",
            "archive_name": "ProbeBrowser_{version}_{date}.7z",
            "release": {
                "tag": "ProbeBrowser-v{version}",
                "title": "{display_name} {version} Portable",
                "asset_match": "ProbeBrowser",
                "version_pattern": "\u{7248}\u{672c}: (\\d+(?:\\.\\d+)+)"
            }
        })
    }

    #[test]
    fn format_value_substitutes_bare_fields() {
        let context = vec![
            ("display_name".to_string(), "Probe Browser".to_string()),
            ("version".to_string(), "153.1.95.102".to_string()),
        ];
        assert_eq!(
            format_value("{display_name} {version}", &context).unwrap(),
            "Probe Browser 153.1.95.102"
        );
        assert_eq!(format_value("{{literal}}", &context).unwrap(), "{literal}");
        assert_eq!(format_value("no fields", &context).unwrap(), "no fields");
    }

    #[test]
    fn format_value_missing_key_raises_like_keyerror() {
        let context = vec![("a".to_string(), "1".to_string())];
        let err = format_value("{missing}", &context).unwrap_err();
        assert_eq!(err.to_string(), "KeyError: 'missing'");
    }

    #[test]
    fn extract_version_finds_four_part_runs() {
        assert_eq!(
            extract_version("ProbeBrowser-v153.1.95.102"),
            Some("153.1.95.102".to_string())
        );
        assert_eq!(extract_version(""), None);
        assert_eq!(extract_version("no digits here"), None);
        // Three-part versions do not match the 4-dot regex.
        assert_eq!(extract_version("v1.2.3"), None);
        // re.search semantics: the match can start mid-string and must not
        // be fooled by a 3-part prefix of a longer run.
        assert_eq!(extract_version("1.2.3.4.5"), Some("1.2.3.4.5".to_string()));
    }

    #[test]
    fn archive_name_regex_matches_python_pattern_text() {
        // Python: ^ProbeBrowser_\d+(?:\.\d+)+_\d{4}-\d{2}-\d{2}\.7z$
        // (golden: verify_archive_locate_golden.json regex_pattern).
        assert_eq!(
            archive_name_regex_text(&probe_target()).unwrap(),
            "^ProbeBrowser_\\d+(?:\\.\\d+)+_\\d{4}-\\d{2}-\\d{2}\\.7z$"
        );
    }

    #[test]
    fn archive_name_regex_fullmatches_generated_names() {
        let target = probe_target();
        let pattern = archive_name_regex_text(&target).unwrap();
        let compiled = compile_pattern(&format!("^{pattern}$")).unwrap();
        assert!(compiled.is_match("ProbeBrowser_153.1.95.102_2026-07-01.7z"));
        assert!(!compiled.is_match("prefix ProbeBrowser_153.1.95.102_2026-07-01.7z"));
        assert!(!compiled.is_match("ProbeBrowser_v153_2026-07-01.7z"));
        // Case-insensitive compile (Python re.IGNORECASE).
        assert!(compiled.is_match("probebrowser_153.1.95.102_2026-07-01.7z"));
    }

    #[test]
    fn asset_matchers_chain_order_and_fallback() {
        // probe target has archive_name + asset_match: two matchers, in order.
        let target = probe_target();
        let matchers = asset_matchers(&target).unwrap();
        assert_eq!(
            matchers
                .iter()
                .map(|m| m.description.as_str())
                .collect::<Vec<_>>(),
            vec![
                "archive_name:ProbeBrowser_{version}_{date}.7z",
                "asset_match(contains):ProbeBrowser"
            ]
        );

        // No release block, no archive_name -> default(contains) on output_dir.
        let bare = json!({"name": "Zeta", "output_dir": "ZetaDir"});
        let matchers = asset_matchers(&bare).unwrap();
        assert_eq!(matchers.len(), 1);
        assert_eq!(matchers[0].description, "default(contains):ZetaDir");
        assert!((matchers[0].predicate)("zetaDIR_x.7z"));
    }

    #[test]
    fn asset_match_modes() {
        let base = |mode: &str| {
            json!({
                "name": "X",
                "release": {"asset_match": "Stable", "asset_match_mode": mode}
            })
        };
        let cases: [(&str, &str, &str); 5] = [
            ("exact", "stable", "stablex"),
            ("prefix", "stableX.7z", "xstable.7z"),
            ("suffix", "x-Stable", "stable-x"),
            ("contains", "aStableB", "aStablB"),
            ("regex", "xStabley", "aStablB"),
        ];
        for (mode, hit, miss) in cases {
            let matchers = asset_matchers(&base(mode)).unwrap();
            assert_eq!(matchers.len(), 1, "mode {mode}");
            assert!(
                (matchers[0].predicate)(hit),
                "mode {mode} should match {hit}"
            );
            assert!(
                !(matchers[0].predicate)(miss),
                "mode {mode} should not match {miss}"
            );
        }
    }

    #[test]
    fn target_match_description_joins_or_reports_none() {
        assert!(target_match_description(&probe_target())
            .unwrap()
            .contains(", "));
        let empty = json!({"name": ""});
        assert_eq!(target_match_description(&empty).unwrap(), "(none)");
    }

    #[test]
    fn render_release_matches_release_render_reference() {
        // Golden: _migration/pe-golden/release_render_reference.json - tag/title
        // byte-equal and the body file written under a temp workdir.
        let golden_path = Path::new("_migration/pe-golden/release_render_reference.json");
        if !golden_path.exists() {
            eprintln!("golden not present in this checkout; skipping");
            return;
        }
        let golden: Value =
            serde_json::from_str(&std::fs::read_to_string(golden_path).unwrap()).unwrap();

        let target = probe_target();
        let dir = std::env::temp_dir().join(format!("pe-release-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let workdir = dir.join("work");

        let rendered = render_release(&target, &workdir, Some("153.1.95.102"), Some("2026-07-01"))
            .expect("render_release");

        assert_eq!(rendered["tag"], golden["rendered"]["tag"]);
        assert_eq!(rendered["title"], golden["rendered"]["title"]);
        assert_eq!(
            rendered.as_object().unwrap().len(),
            golden["rendered_keys"].as_array().unwrap().len()
        );

        let body_path = rendered["body_path"].as_str().unwrap();
        let body = std::fs::read_to_string(body_path).unwrap();
        assert_eq!(
            body,
            "\u{81ea}\u{52a8}\u{6784}\u{5efa}\u{7684} Probe Browser \u{4fbf}\u{643a}\u{7248}\n\n\u{6784}\u{5efa}\u{65f6}\u{95f4}: 2026-07-01\nProbe Browser \u{7248}\u{672c}: 153.1.95.102\n"
        );

        // Body file carries the version the version_pattern reads back.
        assert_body_versions(
            &body,
            &[(
                "probe_stable".to_string(),
                Some(
                    r"\u{7248}\u{672c}: (\d+(?:\.\d+)+)"
                        .replace("\\u{7248}", "\u{7248}")
                        .replace("\\u{672c}", "\u{672c}"),
                ),
                Some("153.1.95.102".to_string()),
            )],
        )
        .unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assert_body_versions_reports_mismatch_like_python() {
        let pattern = "\u{7248}\u{672c}: (\\d+(?:\\.\\d+)+)".to_string();
        let err = assert_body_versions(
            "\u{7248}\u{672c}: 1.0.0.0",
            &[(
                "chrome_stable".to_string(),
                Some(pattern.clone()),
                Some("2.0.0.0".to_string()),
            )],
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("read '1.0.0.0', expected '2.0.0.0'"));
        assert!(err
            .to_string()
            .contains("Release body and version_pattern disagree"));

        let err = assert_body_versions(
            "no match here",
            &[(
                "chrome_stable".to_string(),
                Some(pattern),
                Some("2.0.0.0".to_string()),
            )],
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("matches nothing in the rendered body"));
    }

    #[test]
    fn render_asset_name_matches_reference_asset_name() {
        // Golden asset_name: ProbeBrowser-153.1.95.102-x64-2026-07-01 -
        // produced from the reference input's archive_name template (the
        // hyphen variant recorded in release_render_reference.json).
        let target = json!({
            "target": "probe_stable",
            "display_name": "ProbeBrowser",
            "architecture": "x64",
            "archive_name": "{display_name}-{version}-{arch}-{date}"
        });
        assert_eq!(
            render_asset_name(&target, "153.1.95.102", "2026-07-01", None).unwrap(),
            "ProbeBrowser-153.1.95.102-x64-2026-07-01"
        );
        // Default template from release.py when archive_name is absent.
        let bare = json!({"display_name": "ProbeBrowser", "architecture": "x64"});
        assert_eq!(
            render_asset_name(&bare, "1.0.0.0", "2026-07-01", None).unwrap(),
            "ProbeBrowser_Portable_1.0.0.0_2026-07-01.7z"
        );
    }

    #[test]
    fn github_headers_token_optional() {
        // No token -> Accept only. Token set -> Authorization appended.
        let saved = std::env::var("GITHUB_TOKEN").ok();
        std::env::remove_var("GITHUB_TOKEN");
        let headers = github_headers();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "Accept");

        std::env::set_var("GITHUB_TOKEN", "t0ken");
        let headers = github_headers();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[1].1, "token t0ken");

        // Empty token is falsy in Python: no header.
        std::env::set_var("GITHUB_TOKEN", "");
        let headers = github_headers();
        assert_eq!(headers.len(), 1);

        match saved {
            Some(value) => std::env::set_var("GITHUB_TOKEN", value),
            None => std::env::remove_var("GITHUB_TOKEN"),
        }
    }

    #[test]
    fn build_context_keys_match_builder_py() {
        let target = probe_target();
        let context = build_context(&target, "1.2.3.4", "2026-07-01", "1.2.3.4");
        let keys: Vec<&str> = context.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "target",
                "name",
                "display_name",
                "output_dir",
                "version",
                "package_version",
                "date",
                "arch",
                "archive",
                "sha256",
                "size",
                "chrome_plus_version",
                "run_url"
            ]
        );
        let get = |key: &str| {
            context
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(get("name"), "Probe");
        assert_eq!(get("display_name"), "Probe Browser");
        assert_eq!(get("output_dir"), "ProbeBrowser");
        assert_eq!(get("arch"), "x64");
    }
}

#[cfg(test)]
mod decision_tree_tests {
    use super::*;

    #[test]
    fn decision_tree_replays_five_branches_with_fake_env() {
        // Golden replay of the check decision tree
        // (release_check_contract.md): the branch order and the resulting
        // UPDATE_NEEDED flag for each of the five branches.
        // 1. force_build (manual dispatch) wins over everything else.
        let (update, message) = decide_update_needed(true, None, false, "2.0.0.0");
        assert!(update);
        assert_eq!(message, "Manual dispatch detected; forcing build.");
        // force beats an up-to-date state too.
        let (update, message) =
            decide_update_needed(true, Some("153.1.95.102"), true, "153.1.95.102");
        assert!(update);
        assert_eq!(message, "Manual dispatch detected; forcing build.");

        // 2. no current version.
        let (update, message) = decide_update_needed(false, None, false, "153.1.95.102");
        assert!(update);
        assert_eq!(
            message,
            "No existing release version found; build is needed."
        );

        // 3. asset missing.
        let (update, message) = decide_update_needed(false, Some("1.0.0.0"), false, "153.1.95.102");
        assert!(update);
        assert_eq!(message, "Release asset is missing; rebuild is needed.");

        // 4. upgrade detected.
        let (update, message) = decide_update_needed(false, Some("1.0.0.0"), true, "2.0.0.0");
        assert!(update);
        assert_eq!(message, "Version upgrade detected");

        // 5. no newer upstream (equal or downgrade).
        let (update, message) = decide_update_needed(false, Some("2.0.0.0"), true, "2.0.0.0");
        assert!(!update);
        assert_eq!(message, "No newer upstream version detected.");
        let (update, message) = decide_update_needed(false, Some("2.0.0.0"), true, "1.9.9");
        assert!(!update);
        assert_eq!(message, "No newer upstream version detected.");
    }
}
