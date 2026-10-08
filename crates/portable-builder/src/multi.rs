//! Port of portable_builder/multi.py - multi-target orchestration. Owner: Wave3.
//!
//! Porting notes live in docs/MIGRATION_RUST_TAURI.md. This module is a stub
//! until its owning wave lands; see the wave table in S9.
//! Port of portable_builder/multi.py - multi-target orchestration. Owner: Wave3-B.
//!
//! Semantics follow the Python source exactly (multi_contract.md + both
//! addenda are the contract of record):
//! - env_name: [^A-Za-z0-9]+ -> "_", strip('_'), upper;
//! - split_targets: comma split + strip, order-preserving (no dedup/sort);
//! - asset_facts: {prefix}_ARCHIVE/_SHA256/_SIZE with a release-asset fallback
//!   (digest "sha256:<hex>" split on the first colon), human_size rendering,
//!   empty -> "-";
//! - build_flat_context: lowercased env_name prefix feeding the multi release
//!   body placeholders;
//! - validate_target_asset_matchers: dry-run sample (123.456.789.0 /
//!   2099-12-31) rendered archive names cross-tested against every other
//!   target's matchers - overlap = RuntimeError;
//! - {prefix}_UPDATE crosses check -> build -> update steps through GITHUB_ENV
//!   (real env vars, no in-process state);
//! - render_multi_release falls back to single render_release when the config
//!   has no top-level release block; tag/title/body defaults differ from the
//!   single path ("v{date}" / "{date}" / "");
//! - update_multi_release only deletes assets of targets rebuilt this run
//!   (UPDATE=true); PATCH body semantics match the single-target path.
//!
//! Asset UPLOAD stays out of scope (workflow softprops boundary).
//!
//! Porting notes live in docs/MIGRATION_RUST_TAURI.md section 4.6.

use std::path::Path;

use anyhow::{bail, Result};
use regex::RegexBuilder;
use serde_json::{json, Value};

use crate::config::get_target;
use crate::github_env::{build_run_url, write_env};
use crate::log_fmt::{info, warn};
use crate::release::{
    assert_body_versions, delete_target_assets, download_release_asset, find_latest_target_asset,
    format_value, get_version_info, github_headers, latest_release, render_release,
    target_match_description, target_release_config, today,
};
use crate::tools::{human_size, sha256_file};
use crate::versions::{is_upgrade, should_create_new_release};

/// Port of multi.py::env_name.
pub fn env_name(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_underscore = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    // Python: re.sub then .strip("_") then .upper().
    let trimmed = out.trim_matches('_');
    trimmed.to_uppercase()
}

/// Port of multi.py::split_targets - comma split + strip + drop empties,
/// order preserved (no dedup, no sort).
pub fn split_targets(targets: &str) -> Vec<String> {
    targets
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// Python re.search(pattern, text, re.IGNORECASE) group(1); None either when
/// there is no text/pattern or no match.
pub fn extract_with_pattern(text: &str, pattern: Option<&str>) -> Option<String> {
    let pattern = pattern?;
    if text.is_empty() || pattern.is_empty() {
        return None;
    }
    let compiled = RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .ok()?;
    compiled
        .captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// multi.py env-prefix resolution: target.get("env_prefix") or
/// env_name(target_name).
fn env_prefix_of(target: &Value, target_name: &str) -> String {
    target
        .get("env_prefix")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| env_name(target_name))
}

/// Read an env var as Python os.getenv(name, "") - missing maps to empty.
fn env_or_empty(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

/// Port of multi.py::asset_facts - archive / digest / size for a target,
/// however this run produced them. A digest-less target with a release falls
/// back to the release's latest matching asset.
pub fn asset_facts(config: &Value, target_name: &str, release_id: Option<i64>) -> Result<Value> {
    let target = get_target(config, target_name)?;
    let prefix = env_prefix_of(&target, target_name);

    let mut name = env_or_empty(&format!("{prefix}_ARCHIVE"));
    let mut digest = env_or_empty(&format!("{prefix}_SHA256"));
    let mut size = env_or_empty(&format!("{prefix}_SIZE"));

    if digest.is_empty() {
        if let Some(release_id) = release_id {
            let asset = match find_latest_target_asset(release_id, &target) {
                Ok(asset) => asset,
                Err(exc) => {
                    warn(format!(
                        "Could not read the existing release asset for {target_name}: {exc:#}"
                    ));
                    None
                }
            };
            if let Some(asset) = asset {
                if name.is_empty() {
                    name = asset
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                }
                if size.is_empty() {
                    size = asset
                        .get("size")
                        .map(|value| match value {
                            Value::Number(number) => number.to_string(),
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default();
                }
                // GitHub reports asset digests as 'sha256:<hex>' - take the
                // part after the first colon.
                let raw = asset
                    .get("digest")
                    .map(asset_string_or_json)
                    .unwrap_or_default();
                digest = raw
                    .split_once(':')
                    .map(|(_, after)| after)
                    .unwrap_or("")
                    .to_string();
            }
        }
    }

    Ok(json!({
        "archive": name,
        "sha256": digest,
        "size": human_size(if size.is_empty() { None } else { size.parse::<f64>().ok() }),
    }))
}

/// Python str(value or "") over a JSON field.
fn asset_string_or_json(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Port of multi.py::build_flat_context - the lowercase-prefixed flat context
/// feeding multi release body templates.
pub fn build_flat_context(
    config: &Value,
    target_names: &[String],
    packages: &[(String, String)],
    build_date: &str,
) -> Result<Vec<(String, String)>> {
    let mut context: Vec<(String, String)> = vec![
        ("date".to_string(), build_date.to_string()),
        (
            "chrome_plus_version".to_string(),
            env_or_empty("CHROME_PLUS_VERSION"),
        ),
        ("run_url".to_string(), build_run_url()),
    ];
    let release_id = env_or_empty("RELEASE_ID");
    let release_id = release_id.parse::<i64>().ok();
    for target_name in target_names {
        let package_version = packages
            .iter()
            .find(|(name, _)| name == target_name)
            .map(|(_, version)| version.clone())
            .unwrap_or_default();
        let prefix = env_name(target_name).to_lowercase();
        context.push((format!("{prefix}_version"), package_version));
        let facts = asset_facts(config, target_name, release_id)?;
        for key in ["archive", "sha256", "size"] {
            let value = facts
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // Python: value or "-" (empty renders as the dash placeholder).
            context.push((
                format!("{prefix}_{key}"),
                if value.is_empty() {
                    "-".to_string()
                } else {
                    value
                },
            ));
        }
    }
    Ok(context)
}

/// Port of multi.py::release_asset_present.
pub fn release_asset_present(
    config: &Value,
    target_name: &str,
    release_id: Option<i64>,
) -> Result<bool> {
    let Some(release_id) = release_id else {
        return Ok(false);
    };
    let target = get_target(config, target_name)?;
    Ok(find_latest_target_asset(release_id, &target)?.is_some())
}

/// Port of multi.py::validate_target_asset_matchers - dry-run sample render
/// of every target's archive_name, cross-tested against the other targets'
/// matchers. Overlap = RuntimeError (protects against one target's release
/// sweeping another target's assets).
pub fn validate_target_asset_matchers(config: &Value, target_names: &[String]) -> Result<()> {
    let mut sample_names: Vec<(String, String)> = Vec::new();
    for target_name in target_names {
        let target = get_target(config, target_name)?;
        let Some(archive_name) = target.get("archive_name").and_then(Value::as_str) else {
            continue;
        };
        let context = vec![
            ("target".to_string(), target_name.clone()),
            (
                "name".to_string(),
                target
                    .get("name")
                    .and_then(Value::as_str)
                    .or_else(|| target.get("display_name").and_then(Value::as_str))
                    .unwrap_or("")
                    .to_string(),
            ),
            (
                "display_name".to_string(),
                target
                    .get("display_name")
                    .and_then(Value::as_str)
                    .or_else(|| target.get("name").and_then(Value::as_str))
                    .unwrap_or("")
                    .to_string(),
            ),
            (
                "output_dir".to_string(),
                target
                    .get("output_dir")
                    .and_then(Value::as_str)
                    .or_else(|| target.get("name").and_then(Value::as_str))
                    .unwrap_or("Browser")
                    .to_string(),
            ),
            ("version".to_string(), "123.456.789.0".to_string()),
            ("package_version".to_string(), "123.456.789.0".to_string()),
            ("date".to_string(), "2099-12-31".to_string()),
            (
                "arch".to_string(),
                target
                    .get("architecture")
                    .and_then(Value::as_str)
                    .unwrap_or("x64")
                    .to_string(),
            ),
        ];
        sample_names.push((target_name.clone(), format_value(archive_name, &context)?));
    }

    let mut conflicts: Vec<String> = Vec::new();
    for owner_name in target_names {
        let owner = get_target(config, owner_name)?;
        let matchers = crate::release::asset_matchers(&owner)?;
        for (sample_name, produced_name) in &sample_names {
            if owner_name == sample_name {
                continue;
            }
            if matchers.iter().any(|m| (m.predicate)(produced_name)) {
                conflicts.push(format!(
                    "{owner_name} matcher ({}) also matches {sample_name} archive '{produced_name}'",
                    target_match_description(&owner)?
                ));
            }
        }
    }

    if !conflicts.is_empty() {
        bail!(
            "Overlapping release asset matchers detected: {}",
            conflicts.join("; ")
        );
    }
    Ok(())
}

/// Port of multi.py::check_targets - the multi-target check loop. Writes the
/// shared keys plus per-target {prefix}_UPDATE and UPSTREAM_{prefix}.
pub fn check_targets(
    config: &Value,
    target_names: &[String],
    workdir: &Path,
) -> Result<Vec<(String, String)>> {
    let event_name = std::env::var("GITHUB_EVENT_NAME").unwrap_or_default();
    let force_build = event_name == "workflow_dispatch";
    let release = latest_release()?;
    let release_body = release
        .as_ref()
        .and_then(|r| r.get("body"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let release_id = release
        .as_ref()
        .and_then(|r| r.get("id"))
        .and_then(Value::as_i64);
    let release_tag = release
        .as_ref()
        .and_then(|r| r.get("tag_name"))
        .and_then(Value::as_str)
        .map(str::to_string);

    validate_target_asset_matchers(config, target_names)?;

    let mut packages: Vec<(String, String)> = Vec::new();
    let mut updates: Vec<(String, bool)> = Vec::new();
    let mut current_versions: Vec<(String, Option<String>)> = Vec::new();
    for target_name in target_names {
        let target = get_target(config, target_name)?;
        let package = get_version_info(&target, Some(workdir))?;
        let upstream_version = package
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        packages.push((target_name.clone(), upstream_version.clone()));

        let release_config = target_release_config(&target);
        let current = extract_with_pattern(
            &release_body,
            release_config
                .get("version_pattern")
                .and_then(Value::as_str),
        );
        current_versions.push((target_name.clone(), current.clone()));
        let asset_present = release_asset_present(config, target_name, release_id)?;
        let update = force_build
            || current.is_none()
            || !asset_present
            || (upstream_version != current.clone().unwrap_or_default()
                && is_upgrade(&upstream_version, current.as_deref().unwrap_or("")));
        updates.push((target_name.clone(), update));
        info(format!(
            "{target_name}: upstream={upstream_version} current={} asset_present={asset_present} update={}",
            current.as_deref().unwrap_or("None"),
            bool_py(update)
        ));
    }

    let config_release = config.get("release").cloned().unwrap_or_else(|| json!({}));
    let tag_target = config_release
        .get("tag_target")
        .and_then(Value::as_str)
        .unwrap_or(&target_names[0])
        .to_string();
    let tag_target_config = get_target(config, &tag_target)?;
    let create_policy = config_release
        .get("create_new_release_on")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            tag_target_config
                .get("release")
                .and_then(|r| r.get("create_new_release_on"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    let tag_target_update = updates
        .iter()
        .find(|(name, _)| *name == tag_target)
        .map(|(_, update)| *update)
        .unwrap_or(false);
    let tag_target_current = current_versions
        .iter()
        .find(|(name, _)| *name == tag_target)
        .and_then(|(_, current)| current.clone());
    let tag_target_upstream = packages
        .iter()
        .find(|(name, _)| *name == tag_target)
        .map(|(_, version)| version.clone())
        .unwrap_or_default();

    let mut create_new_release = release_id.is_none();
    if release_id.is_some() && tag_target_update {
        create_new_release = tag_target_current.is_some()
            && should_create_new_release(
                create_policy.as_deref(),
                &tag_target_upstream,
                tag_target_current.as_deref().unwrap_or(""),
            )?;
    }

    // In-place tag/title rewrite only when we deliberately stayed on the same
    // release; a create_new_release run publishes a fresh tag instead.
    let mut minor_update = false;
    if release_id.is_some() && tag_target_update && !create_new_release {
        minor_update = tag_target_current.is_some()
            && tag_target_upstream != tag_target_current.clone().unwrap_or_default()
            && is_upgrade(
                &tag_target_upstream,
                tag_target_current.as_deref().unwrap_or(""),
            );
    }

    let mut values: Vec<(String, String)> = vec![
        (
            "UPDATE_NEEDED".to_string(),
            bool_py(updates.iter().any(|(_, update)| *update)),
        ),
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

    for (target_name, upstream_version) in &packages {
        let target = get_target(config, target_name)?;
        let prefix = env_prefix_of(&target, target_name);
        let update = updates
            .iter()
            .find(|(name, _)| name == target_name)
            .map(|(_, update)| *update)
            .unwrap_or(false);
        values.push((format!("{prefix}_UPDATE"), bool_py(update)));
        values.push((format!("UPSTREAM_{prefix}"), upstream_version.clone()));
    }

    write_env(&values)?;
    Ok(values)
}

/// Python str(bool): "true" / "false".
fn bool_py(value: bool) -> String {
    if value {
        "true".to_string()
    } else {
        "false".to_string()
    }
}

/// Port of multi.py::build_selected_targets - the build loop. Reads
/// {prefix}_UPDATE from the environment (cross-step GITHUB_ENV protocol),
/// workflow_dispatch forces every target, writes the six result keys per
/// built target, and ends with ensure_shared_release_assets.
///
/// NOTE: build_target / archive_target live in builder.rs (Wave3-A). The
/// engine build/archive primitives are invoked through the same module; this
/// function mirrors Python multi.py's loop shape one-for-one.
pub fn build_selected_targets(
    config: &Value,
    target_names: &[String],
    workdir: &Path,
    builder_dir: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let mut built: Vec<(String, String)> = Vec::new();
    for target_name in target_names {
        let target = get_target(config, target_name)?;
        let prefix = env_prefix_of(&target, target_name);
        let mut should_build = env_or_empty(&format!("{prefix}_UPDATE")).to_lowercase() == "true";
        if std::env::var("GITHUB_EVENT_NAME").unwrap_or_default() == "workflow_dispatch" {
            should_build = true;
        }
        if !should_build {
            info(format!("Skipping {target_name}; no update required."));
            continue;
        }

        let result = crate::builder::build_target(&target, workdir, builder_dir)?;
        let package_version = result
            .get("package_version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let archive = crate::builder::archive_target(
            &target,
            workdir,
            Some(&package_version),
            None,
            Some(&package_version),
            None,
        )?;
        built.push((target_name.clone(), package_version.clone()));
        write_env(&[
            (format!("{prefix}_VERSION"), package_version.clone()),
            (
                format!("{prefix}_BUILD_VERSION"),
                result
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ),
            (format!("{prefix}_PACKAGE_VERSION"), package_version),
            (
                format!("{prefix}_ARCHIVE"),
                archive
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ),
            (
                format!("{prefix}_SHA256"),
                archive
                    .get("sha256")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ),
            (
                format!("{prefix}_SIZE"),
                archive
                    .get("size")
                    .map(asset_string_or_json)
                    .unwrap_or_default(),
            ),
        ])?;
    }
    ensure_shared_release_assets(config, target_names, workdir)?;
    Ok(built)
}

/// Port of multi.py::ensure_shared_release_assets - only targets NOT rebuilt
/// this run ({prefix}_UPDATE != true) carry their previous release asset
/// forward into build/assets, then publish the carried facts.
pub fn ensure_shared_release_assets(
    config: &Value,
    target_names: &[String],
    workdir: &Path,
) -> Result<Vec<String>> {
    let release_id = env_or_empty("RELEASE_ID");
    let create_new_release = env_or_empty("CREATE_NEW_RELEASE").to_lowercase() == "true";
    if release_id.is_empty() || !create_new_release {
        return Ok(Vec::new());
    }
    let release_id: i64 = release_id.parse()?;

    let assets_dir = workdir.join("build").join("assets");
    std::fs::create_dir_all(&assets_dir)?;
    let mut carried: Vec<String> = Vec::new();
    for target_name in target_names {
        let target = get_target(config, target_name)?;
        let prefix = env_prefix_of(&target, target_name);
        let should_build = env_or_empty(&format!("{prefix}_UPDATE")).to_lowercase() == "true";
        if should_build {
            continue;
        }

        let Some(previous_asset) = find_latest_target_asset(release_id, &target)? else {
            warn(format!(
                "No previous release asset found for {target_name}; shared release will not carry one forward."
            ));
            continue;
        };

        let asset_name = previous_asset
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let destination = assets_dir.join(&asset_name);
        if destination.exists() {
            info(format!(
                "Reusing carried asset already present: {}",
                destination
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
            ));
        } else {
            info(format!(
                "Carrying forward previous asset for {target_name}: {asset_name}"
            ));
            download_release_asset(&previous_asset, &destination)?;
        }
        carried.push(destination.to_string_lossy().into_owned());

        // Publish the carried asset's facts too, so the release notes can
        // show a checksum for a channel that was not rebuilt in this run.
        write_env(&[
            (format!("{prefix}_ARCHIVE"), asset_name),
            (format!("{prefix}_SHA256"), sha256_file(&destination)?),
            (
                format!("{prefix}_SIZE"),
                std::fs::metadata(&destination)?.len().to_string(),
            ),
        ])?;
    }
    Ok(carried)
}

/// Port of multi.py::render_multi_release - config-level release block
/// missing => degrade to the single-target render_release; version fallback
/// {prefix}_PACKAGE_VERSION > {prefix}_VERSION > UPSTREAM_{prefix}; tag /
/// title / body defaults "v{date}" / "{date}" / "" (different from single!).
pub fn render_multi_release(
    config: &Value,
    target_names: &[String],
    workdir: &Path,
) -> Result<Value> {
    let Some(release_config) = config.get("release").filter(|value| !value.is_null()) else {
        let first_target = get_target(config, &target_names[0])?;
        return render_release(&first_target, workdir, None, None);
    };

    let build_date = match std::env::var("BUILD_DATE") {
        Ok(value) if !value.is_empty() => value,
        _ => today(),
    };
    let mut packages: Vec<(String, String)> = Vec::new();
    for target_name in target_names {
        let target = get_target(config, target_name)?;
        let prefix = env_prefix_of(&target, target_name);
        let version = ["PACKAGE_VERSION", "VERSION"]
            .iter()
            .map(|suffix| env_or_empty(&format!("{prefix}_{suffix}")))
            .find(|value| !value.is_empty())
            .unwrap_or_else(|| env_or_empty(&format!("UPSTREAM_{prefix}")));
        packages.push((target_name.clone(), version));
    }

    let context = build_flat_context(config, target_names, &packages, &build_date)?;
    let tag = format_value(
        release_config
            .get("tag")
            .and_then(Value::as_str)
            .unwrap_or("v{date}"),
        &context,
    )?;
    let title = format_value(
        release_config
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("{date}"),
        &context,
    )?;
    let body = format_value(
        release_config
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or(""),
        &context,
    )?;

    let mut expectations: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    for (target_name, version) in &packages {
        let target = get_target(config, target_name)?;
        let pattern = target_release_config(&target)
            .get("version_pattern")
            .and_then(Value::as_str)
            .map(str::to_string);
        expectations.push((target_name.clone(), pattern, Some(version.clone())));
    }
    assert_body_versions(&body, &expectations)?;

    let body_path = workdir.join("build").join("release_body.md");
    if let Some(parent) = body_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&body_path, &body)?;

    write_env(&[
        ("RELEASE_TAG".to_string(), tag.clone()),
        ("RELEASE_TITLE".to_string(), title.clone()),
        (
            "RELEASE_BODY_PATH".to_string(),
            body_path.to_string_lossy().into_owned(),
        ),
    ])?;
    Ok(json!({
        "tag": tag,
        "title": title,
        "body_path": body_path.to_string_lossy(),
    }))
}

/// Port of multi.py::update_multi_release - only rebuilt targets get their
/// old assets deleted (UPDATE=true); PATCH body semantics match the
/// single-target path (MINOR_UPDATE adds name+tag_name).
pub fn update_multi_release(config: &Value, target_names: &[String], workdir: &Path) -> Result<()> {
    let release_id = env_or_empty("RELEASE_ID");
    if release_id.is_empty() {
        info("No RELEASE_ID set; create release step will handle publishing.");
        return Ok(());
    }
    let release_id: i64 = release_id.parse()?;

    let release_info = render_multi_release(config, target_names, workdir)?;
    for target_name in target_names {
        let target = get_target(config, target_name)?;
        let prefix = env_prefix_of(&target, target_name);
        if env_or_empty(&format!("{prefix}_UPDATE")).to_lowercase() == "true" {
            delete_target_assets(release_id, &target)?;
        }
    }

    let repo = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
    let body = std::fs::read_to_string(
        release_info
            .get("body_path")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    let mut data = json!({ "body": body });
    if env_or_empty("MINOR_UPDATE").to_lowercase() == "true" {
        data["name"] = release_info.get("title").cloned().unwrap_or_default();
        data["tag_name"] = release_info.get("tag").cloned().unwrap_or_default();
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let mut request = client
        .patch(format!(
            "https://api.github.com/repos/{repo}/releases/{release_id}"
        ))
        .json(&data);
    for (name, value) in github_headers() {
        request = request.header(name, value);
    }
    request.send()?.error_for_status()?;
    info(format!("Updated release {release_id}."));
    Ok(())
}

// Port of tools.py::human_size passthrough is unused here; kept for parity
// with the Python import list is unnecessary - dead code would trip clippy.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_name_matches_multi_reference() {
        // Golden: _migration/pe-golden/multi_reference.json env_name_cases.
        let golden_path = std::path::Path::new("_migration/pe-golden/multi_reference.json");
        if !golden_path.exists() {
            eprintln!("golden not present in this checkout; skipping");
            return;
        }
        let golden: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(golden_path).unwrap()).unwrap();
        for (input, expected) in golden["env_name_cases"].as_object().unwrap() {
            let got = env_name(input);
            assert_eq!(
                got,
                expected.as_str().unwrap(),
                "env_name({input:?}) diverges from Python"
            );
        }
    }

    #[test]
    fn split_targets_matches_multi_reference() {
        // Golden: multi_reference.json split_targets (order preserved).
        let golden_path = std::path::Path::new("_migration/pe-golden/multi_reference.json");
        if !golden_path.exists() {
            eprintln!("golden not present in this checkout; skipping");
            return;
        }
        let golden: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(golden_path).unwrap()).unwrap();
        let cases = golden["split_targets"].as_object().unwrap();
        let inputs = [
            ("basic", "chrome_stable, edge_stable, helium_stable"),
            ("single", "chrome_stable"),
        ];
        for (key, input) in inputs {
            let expected: Vec<String> = cases[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            assert_eq!(split_targets(input), expected, "split_targets({input:?})");
        }
        // Comma-split with empties filtered, order kept, no dedup.
        assert_eq!(
            split_targets("a,, b ,"),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(split_targets("a,a"), vec!["a".to_string(), "a".to_string()]);
        assert!(split_targets(" , ").is_empty());
    }

    #[test]
    fn extract_with_pattern_group_semantics() {
        let pattern = r"Chrome: (\d+(?:\.\d+)+)";
        assert_eq!(
            extract_with_pattern("Chrome: 153.1.95.102", Some(pattern)),
            Some("153.1.95.102".to_string())
        );
        // Case-insensitive compile (Python re.IGNORECASE).
        assert_eq!(
            extract_with_pattern("chrome: 1.2.3.4", Some(pattern)),
            Some("1.2.3.4".to_string())
        );
        assert_eq!(extract_with_pattern("nothing", Some(pattern)), None);
        assert_eq!(extract_with_pattern("", Some(pattern)), None);
        assert_eq!(extract_with_pattern("text", Some("")), None);
        assert_eq!(extract_with_pattern("text", None), None);
    }

    fn flat_config() -> serde_json::Value {
        serde_json::json!({
            "targets": {
                "chrome_stable": {
                    "name": "Chrome",
                    "display_name": "Chrome++",
                    "architecture": "x64",
                    "archive_name": "Chrome++_{version}_{date}.7z"
                },
                "edge_stable": {
                    "name": "Edge",
                    "display_name": "Edge Portable",
                    "architecture": "x64",
                    "archive_name": "EdgePortable_{version}_{date}.7z"
                }
            }
        })
    }

    #[test]
    fn build_flat_context_matches_multi_reference() {
        // Golden: multi_reference.json flat_context (no env facts, no release
        // id -> dashes for archive/sha256/size).
        let golden_path = std::path::Path::new("_migration/pe-golden/multi_reference.json");
        if !golden_path.exists() {
            eprintln!("golden not present in this checkout; skipping");
            return;
        }
        let golden: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(golden_path).unwrap()).unwrap();

        // Single-threaded env mutation: clear the facts and release id.
        let keys = [
            "CHROME_PLUS_VERSION",
            "RELEASE_ID",
            "GITHUB_REPOSITORY",
            "GITHUB_RUN_ID",
        ];
        let saved: Vec<(String, Option<String>)> = keys
            .iter()
            .map(|k| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for k in keys {
            std::env::remove_var(k);
        }

        let config = flat_config();
        let target_names = vec!["chrome_stable".to_string(), "edge_stable".to_string()];
        let packages = vec![
            ("chrome_stable".to_string(), "153.1.95.102".to_string()),
            ("edge_stable".to_string(), "138.0.3351.121".to_string()),
        ];
        let context = build_flat_context(&config, &target_names, &packages, "2026-07-01")
            .expect("build_flat_context");

        let expected = golden["flat_context"].as_object().unwrap();
        for (key, value) in expected {
            let got = context
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            assert_eq!(
                got,
                value.as_str().unwrap(),
                "flat_context[{key}] diverges from Python"
            );
        }

        for (k, v) in saved {
            match v {
                Some(value) => std::env::set_var(&k, value),
                None => std::env::remove_var(&k),
            }
        }
    }

    #[test]
    fn validate_target_asset_matchers_passes_distinct_and_rejects_overlap() {
        // Distinct templates: no conflict (the through scenario).
        validate_target_asset_matchers(
            &flat_config(),
            &["chrome_stable".to_string(), "edge_stable".to_string()],
        )
        .expect("distinct templates must pass");

        // Overlap requires the OWNER's matcher to hit the OTHER target's
        // SAMPLE name. Give edge the same template as chrome: edge's dry-run
        // sample then fullmatches chrome's archive_name matcher.
        let mut overlap = flat_config();
        overlap["targets"]["edge_stable"]["archive_name"] =
            serde_json::json!("Chrome++_{version}_{date}.7z");
        let err = validate_target_asset_matchers(
            &overlap,
            &["chrome_stable".to_string(), "edge_stable".to_string()],
        )
        .expect_err("overlapping matchers must fail");
        let message = err.to_string();
        assert!(
            message.contains("Overlapping release asset matchers detected"),
            "{message}"
        );
        assert!(message.contains("also matches"), "{message}");
    }

    #[test]
    fn flat_context_version_keys_use_lowercase_env_name() {
        // multi.py: prefix = env_name(target_name).lower(); a target id like
        // "chrome-beta" yields "chrome_beta_version" keys.
        let config = serde_json::json!({
            "targets": {
                "chrome-beta": {"name": "ChromeBeta", "display_name": "Chrome Beta"}
            }
        });
        let saved = std::env::var("RELEASE_ID").ok();
        std::env::remove_var("RELEASE_ID");
        let context = build_flat_context(
            &config,
            &["chrome-beta".to_string()],
            &[("chrome-beta".to_string(), "1.2.3.4".to_string())],
            "2026-07-01",
        )
        .unwrap();
        let get = |key: &str| {
            context
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(get("chrome_beta_version"), "1.2.3.4");
        assert_eq!(get("chrome_beta_archive"), "-");
        assert_eq!(get("chrome_beta_sha256"), "-");
        assert_eq!(get("chrome_beta_size"), "-");
        match saved {
            Some(v) => std::env::set_var("RELEASE_ID", v),
            None => std::env::remove_var("RELEASE_ID"),
        }
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    #[test]
    fn render_multi_release_matches_golden() {
        // Golden: _migration/pe-golden/multi_render_golden.json - the config-
        // level release render (flat context, byte-level body with run_url
        // empty locally). Env facts stand in for the built targets.
        let golden_path = std::path::Path::new("_migration/pe-golden/multi_render_golden.json");
        if !golden_path.exists() {
            eprintln!("golden not present in this checkout; skipping");
            return;
        }
        let golden: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(golden_path).unwrap()).unwrap();

        let config = serde_json::json!({
            "release": {
                "tag": "multi-{date}",
                "title": "Portable Browsers {date}",
                "body": "Chrome: {chrome_stable_version} ({chrome_stable_size})\nEdge: {edge_stable_version}\n\u{6784}\u{5efa}\u{65f6}\u{95f4}: {date}\nChrome++: {chrome_plus_version}\n"
            },
            "targets": {
                "chrome_stable": {"name": "Chrome", "display_name": "Chrome++", "architecture": "x64"},
                "edge_stable": {"name": "Edge", "display_name": "Edge Portable", "architecture": "x64"}
            }
        });
        let target_names = vec!["chrome_stable".to_string(), "edge_stable".to_string()];

        // Env vars are process-global: run everything in this one test and
        // restore afterwards.
        let keys = [
            "CHROME_PLUS_VERSION",
            "RELEASE_ID",
            "BUILD_DATE",
            "CHROME_STABLE_PACKAGE_VERSION",
            "CHROME_STABLE_VERSION",
            "UPSTREAM_CHROME_STABLE",
            "CHROME_STABLE_ARCHIVE",
            "CHROME_STABLE_SHA256",
            "CHROME_STABLE_SIZE",
            "EDGE_STABLE_PACKAGE_VERSION",
            "EDGE_STABLE_VERSION",
            "UPSTREAM_EDGE_STABLE",
            "EDGE_STABLE_ARCHIVE",
            "EDGE_STABLE_SHA256",
            "EDGE_STABLE_SIZE",
            "GITHUB_ENV",
            "GITHUB_OUTPUT",
            "GITHUB_REPOSITORY",
            "GITHUB_RUN_ID",
        ];
        let saved: Vec<(String, Option<String>)> = keys
            .iter()
            .map(|k| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for k in keys {
            std::env::remove_var(k);
        }
        std::env::set_var("CHROME_PLUS_VERSION", "1.18.2");
        std::env::set_var("BUILD_DATE", "2026-07-01");
        // 941.9 MB in bytes -> human_size renders the golden value.
        std::env::set_var("CHROME_STABLE_PACKAGE_VERSION", "153.1.95.102");
        std::env::set_var("CHROME_STABLE_SIZE", "987653734");

        let dir = std::env::temp_dir().join(format!("pe-multi-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let workdir = dir.join("work");

        let rendered = render_multi_release(&config, &target_names, &workdir).unwrap();

        assert_eq!(rendered["tag"], golden["rendered"]["tag"]);
        assert_eq!(rendered["title"], golden["rendered"]["title"]);
        let body = std::fs::read_to_string(rendered["body_path"].as_str().unwrap()).unwrap();
        assert_eq!(body, golden["body"].as_str().unwrap());

        for (k, v) in saved {
            match v {
                Some(value) => std::env::set_var(&k, value),
                None => std::env::remove_var(&k),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
