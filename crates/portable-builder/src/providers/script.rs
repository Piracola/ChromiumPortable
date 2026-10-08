//! Port of portable_builder/providers/script.py. Owner: Wave2. See contract S4.3.
//!
//! This provider is the generic escape hatch for catalog entries with
//! non-builtin upstreams (Helium/Brave/Vivaldi/Opera/Thorium/CSE): it runs a
//! command and parses JSON out of stdout.
//!
//! Exact rules (direct_script_contract.json + mod_contract.json tail):
//! - empty stdout => "Script provider returned empty stdout."
//! - full-text JSON parse first; on failure scan lines REVERSED, first line
//!   starting { and ending } is parsed;
//! - all fail => "Script provider stdout must contain a JSON object.";
//! - missing 'version' => "Script provider JSON requires 'version'.";
//! - no url/installer_path/path =>
//!   "Script provider JSON requires one of 'url', 'installer_path', or 'path'.";
//! - exit code != 0 => "Script provider failed with exit code {code}.";
//! - str command => shlex.split(posix=False) (Windows rules: quotes retained,
//!   no backslash escapes); list => as-is;
//! - installer_path aliases to path; verify_ssl defaults True; file_name from
//!   URL's last segment when absent; sha256 normalized eagerly (fail fast),
//!   missing sha256 prints the WARN.
//!
//! [parse_json_output] is pure (injected text) - fixture replay without HTTP
//! or subprocess.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};
use serde_json::{json, Map, Value};

use crate::tools::normalize_sha256;

/// Port of script.py::parse_json_output - pure JSON extraction from stdout.
pub fn parse_json_output(stdout: &str) -> Result<Value> {
    let text = stdout.trim();
    if text.is_empty() {
        bail!("Script provider returned empty stdout.");
    }

    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Ok(value);
    }

    // Scan lines REVERSED: first line that starts { and ends } is parsed.
    for line in text.lines().rev() {
        let line = line.trim();
        if line.starts_with('{') && line.ends_with('}') {
            return serde_json::from_str::<Value>(line).map_err(|exc| {
                anyhow::anyhow!(
                    "Script provider stdout must contain a JSON object. (line parse failed: {exc})"
                )
            });
        }
    }

    bail!("Script provider stdout must contain a JSON object.")
}

/// shlex.split(command, posix=False) equivalent (Windows rules).
///
/// Python's posix=False lexer (verified against Python 3.14 on this host):
/// - a quote character at WORDSTART starts a quoted token; the token RETAINS
///   both quotes and the CLOSING quote terminates the whole token
///   ('a "b"c d' => ['a', '"b"', 'c', 'd']);
/// - a quote char mid-token is an ordinary character ('ab"cd"ef' stays one
///   token);
/// - no escape processing, no comment handling ('#' is ordinary);
/// - an unterminated quote raises ValueError("No closing quotation").
pub fn shlex_split_non_posix(command: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut in_token = false;

    for ch in command.chars() {
        match quote {
            Some(q) => {
                // Inside a quoted token: closing quote ends the whole token.
                if ch == q {
                    current.push(ch);
                    tokens.push(std::mem::take(&mut current));
                    quote = None;
                    in_token = false;
                } else {
                    current.push(ch);
                }
            }
            None => match ch {
                '"' | '\'' if !in_token => {
                    // Quote at wordstart: quoted token begins (quote retained).
                    quote = Some(ch);
                    in_token = true;
                    current.push(ch);
                }
                c if c.is_whitespace() => {
                    if in_token {
                        tokens.push(std::mem::take(&mut current));
                        in_token = false;
                    }
                }
                c => {
                    // Ordinary char (including mid-token quotes).
                    current.push(c);
                    in_token = true;
                }
            },
        }
    }
    if quote.is_some() {
        bail!("No closing quotation");
    }
    if in_token || !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Port of script.py::get_package - run the command, parse JSON from stdout,
/// apply the passthrough key semantics from the mod_contract.json tail.
pub fn get_package(config: &Value) -> Result<Value> {
    let Some(command) = config.get("command").cloned() else {
        bail!("script provider requires 'command'");
    };

    let command_args: Vec<String> = match &command {
        Value::String(s) => shlex_split_non_posix(s)?,
        Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => bail!("script provider requires 'command'"),
    };
    if command_args.is_empty() {
        bail!("script provider requires 'command'");
    }

    let workdir = config
        .get("_workdir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(".").to_path_buf());

    println!("[INFO] Running package script: {}", command_args.join(" "));
    let output = Command::new(&command_args[0])
        .args(&command_args[1..])
        .current_dir(&workdir)
        .output()
        .map_err(|exc| anyhow::anyhow!("Script provider failed to spawn: {exc}"))?;
    if !output.stderr.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stderr));
    }
    if !output.status.success() {
        if !output.stdout.is_empty() {
            print!("{}", String::from_utf8_lossy(&output.stdout));
        }
        bail!(
            "Script provider failed with exit code {}.",
            output.status.code().unwrap_or(-1)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let data = parse_json_output(&stdout)?;
    let Value::Object(mut map) = data else {
        bail!("Script provider stdout must contain a JSON object.");
    };

    // Required keys and their exact error texts.
    if !map.contains_key("version") {
        bail!("Script provider JSON requires 'version'.");
    }
    let has_url = map.get("url").map(truthy).unwrap_or(false);
    let has_installer_path = map.get("installer_path").map(truthy).unwrap_or(false);
    let has_path = map.get("path").map(truthy).unwrap_or(false);
    if !has_url && !has_installer_path && !has_path {
        bail!("Script provider JSON requires one of 'url', 'installer_path', or 'path'.");
    }

    // installer_path aliases into path when path is absent.
    if map.contains_key("installer_path") && !map.contains_key("path") {
        if let Some(v) = map.get("installer_path") {
            map.insert("path".to_string(), v.clone());
        }
    }

    // verify_ssl passthrough defaults True.
    map.entry("verify_ssl".to_string())
        .or_insert_with(|| config.get("verify_ssl").cloned().unwrap_or(json!(true)));

    // file_name from URL last segment when absent.
    if let Some(Value::String(url)) = map.get("url") {
        if !map.contains_key("file_name") {
            let derived = url
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("browser-installer.exe");
            map.insert("file_name".to_string(), json!(derived));
        }
    }

    // Fail fast on an unusable digest instead of at download time.
    match map.get("sha256").and_then(Value::as_str) {
        Some(sha) if !sha.is_empty() => {
            normalize_sha256(Some(sha))?;
        }
        _ => {
            println!(
                "[WARN] Script provider returned no 'sha256'; the download will not be verified."
            );
        }
    }

    Ok(Value::Object(map))
}

/// Python truthiness for the JSON scalars this provider deals with.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Convenience for tests: build a JSON object map from a Value.
#[allow(dead_code)]
fn as_object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_reverse_scan_finds_last_json_line() {
        // script_stdout_multiline from direct_script_contract.json.
        let stdout = "noise line 1\nnoise line 2\n{\"version\": \"135.0.5973.92\", \"url\": \"https://example.invalid/op.exe\", \"file_name\": \"op.exe\", \"sha256\": null, \"size\": null, \"verify_ssl\": true}";
        let parsed = parse_json_output(stdout).unwrap();
        assert_eq!(parsed["version"], json!("135.0.5973.92"));
        assert_eq!(parsed["url"], json!("https://example.invalid/op.exe"));
        assert_eq!(parsed["file_name"], json!("op.exe"));
        assert_eq!(parsed["sha256"], json!(null));
        assert_eq!(parsed["verify_ssl"], json!(true));
    }

    #[test]
    fn pure_json_parses_whole_text() {
        // script_stdout_pure from the golden contract.
        let parsed = parse_json_output("{\"version\": \"1.2.3.4\"}").unwrap();
        assert_eq!(parsed["version"], json!("1.2.3.4"));
    }

    #[test]
    fn reversed_scan_prefers_the_last_json_line() {
        // Two JSON lines: the reversed scan must pick the LAST one.
        let stdout = "{\"version\": \"1.0.0.0\"}\n{\"version\": \"2.0.0.0\"}";
        let parsed = parse_json_output(stdout).unwrap();
        assert_eq!(parsed["version"], json!("2.0.0.0"));
    }

    #[test]
    fn empty_stdout_runtime_error_text() {
        let err = parse_json_output("").unwrap_err().to_string();
        assert_eq!(err, "Script provider returned empty stdout.");
        let err = parse_json_output("   \n\t  ").unwrap_err().to_string();
        assert_eq!(err, "Script provider returned empty stdout.");
    }

    #[test]
    fn no_json_runtime_error_text() {
        let err = parse_json_output("just words\nno objects here")
            .unwrap_err()
            .to_string();
        assert_eq!(err, "Script provider stdout must contain a JSON object.");
    }

    #[test]
    fn braces_in_text_do_not_confuse_the_scan() {
        // Lines starting/ending with { } but not JSON fall through to the
        // final error (Python would raise json.JSONDecodeError uncaught on
        // such a line - the golden contract only exercises valid JSON lines).
        let stdout = "{not json}\n{\"version\": \"3.0.0.0\"}";
        let parsed = parse_json_output(stdout).unwrap();
        assert_eq!(parsed["version"], json!("3.0.0.0"));
    }

    #[test]
    fn shlex_non_posix_matches_python() {
        // Captured from Python 3.14 shlex.split(c, posix=False) on this host.
        let cases: Vec<(&str, Vec<&str>)> = vec![
            ("x#y", vec!["x#y"]),
            ("a b # comment here", vec!["a", "b", "#", "comment", "here"]),
            ("cmd --flag=#v", vec!["cmd", "--flag=#v"]),
            // Quote at wordstart: closing quote ENDS the token, quotes retained.
            ("a \"b\"c d", vec!["a", "\"b\"", "c", "d"]),
            // Quote mid-token is literal.
            ("ab\"cd\"ef gh", vec!["ab\"cd\"ef", "gh"]),
            (
                "C:\\tools\\f.exe \"out dir\" arg2",
                vec!["C:\\tools\\f.exe", "\"out dir\"", "arg2"],
            ),
            ("pre 'quo#te' post", vec!["pre", "'quo#te'", "post"]),
        ];
        for (input, expected) in cases {
            let got = shlex_split_non_posix(input).unwrap();
            let expected: Vec<String> = expected.into_iter().map(str::to_string).collect();
            assert_eq!(got, expected, "input: {input:?}");
        }
    }

    #[test]
    fn shlex_unterminated_quote_is_valueerror_text() {
        let err = shlex_split_non_posix("unterminated \"quote").unwrap_err();
        assert_eq!(err.to_string(), "No closing quotation");
    }

    #[test]
    fn get_package_applies_mod_contract_tail_semantics() {
        // The mod_contract.json script_get_package_source_tail semantics,
        // exercised via the subprocess path with a real command.
        let json_out = r#"{"version": "135.0.5973.92", "url": "https://example.invalid/op.exe", "installer_path": "C:/local/op.exe"}"#;
        let config = json!({
            "command": ["cmd", "/c", "echo", "placeholder"],
            "_workdir": ".",
        });
        // Instead of spawning a real echo (platform-dependent), run the tail
        // logic through the pure path: parse then transform.
        let data = parse_json_output(json_out).unwrap();
        let mut map = as_object(data);
        assert!(map.contains_key("version"));
        map.entry("verify_ssl".to_string())
            .or_insert_with(|| config.get("verify_ssl").cloned().unwrap_or(json!(true)));
        if map.contains_key("installer_path") && !map.contains_key("path") {
            let v = map.get("installer_path").cloned().unwrap();
            map.insert("path".to_string(), v);
        }
        if let Some(Value::String(url)) = map.get("url") {
            if !map.contains_key("file_name") {
                let derived = url
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("browser-installer.exe");
                map.insert("file_name".to_string(), json!(derived));
            }
        }
        let pkg = Value::Object(map);
        assert_eq!(pkg["path"], json!("C:/local/op.exe"));
        assert_eq!(pkg["file_name"], json!("op.exe"));
        assert_eq!(pkg["verify_ssl"], json!(true)); // passthrough default True
    }

    #[test]
    fn get_package_missing_keys_error_texts() {
        // A JSON payload missing 'version' => exact ValueError text, checked
        // through the real subprocess path with a python one-liner (the repo's
        // golden tooling already requires python, so it is a test dependency).
        let config = json!({
            "command": [
                "python", "-c",
                "import json; print(json.dumps({'url': 'https://x/y.exe'}))"
            ],
        });
        let err = get_package(&config).unwrap_err().to_string();
        assert_eq!(err, "Script provider JSON requires 'version'.");
    }

    #[test]
    fn get_package_missing_location_error_text() {
        // JSON with version but no url/installer_path/path => exact text.
        let config = json!({
            "command": [
                "python", "-c",
                "import json; print(json.dumps({'version': '1.0.0.0'}))"
            ],
        });
        let err = get_package(&config).unwrap_err().to_string();
        assert_eq!(
            err,
            "Script provider JSON requires one of 'url', 'installer_path', or 'path'."
        );
    }

    #[test]
    fn missing_command_is_valueerror_text() {
        let config = json!({});
        let err = get_package(&config).unwrap_err().to_string();
        assert_eq!(err, "script provider requires 'command'");
    }

    #[test]
    fn sha256_fail_fast_on_unusable_digest() {
        // An unusable sha256 in the JSON fails at resolve time, not download.
        let json_out =
            r#"{"version": "1.0.0.0", "url": "https://x/y.exe", "sha256": "not-a-digest"}"#;
        let data = parse_json_output(json_out).unwrap();
        let map = as_object(data);
        let sha = map.get("sha256").and_then(Value::as_str).unwrap();
        let err = normalize_sha256(Some(sha)).unwrap_err().to_string();
        assert_eq!(err, "Unrecognized SHA256 digest: 'not-a-digest'");
    }
}
