//! Port of portable_builder/github_env.py - GITHUB_ENV/GITHUB_OUTPUT writer. Owner: Wave1-A.
//!
//! The heredoc delimiter `PORTABLE_BUILDER_EOF` is a frozen CI contract
//! (docs/MIGRATION_RUST_TAURI.md S1/S4.6): it is emitted ONLY for values
//! containing newlines; every other value stays a plain `KEY=value` line.

use std::io::Write;
use std::path::Path;

/// Port of github_env.py::DELIMITER.
///
/// Only needed for values containing newlines; none currently do, but a stray
/// one would otherwise corrupt the whole file.
const DELIMITER: &str = "PORTABLE_BUILDER_EOF";

/// Port of github_env.py::format_pair.
///
/// `None` maps to the empty string (Python: `"" if value is None else
/// str(value)`). Values containing a newline become a
/// `KEY<<PORTABLE_BUILDER_EOF` heredoc block; all other values stay a plain
/// `KEY=value` line.
pub fn format_pair(key: &str, value: Option<&str>) -> Vec<String> {
    let text = value.unwrap_or("");
    if text.contains('\n') {
        vec![
            format!("{key}<<{DELIMITER}"),
            text.to_string(),
            DELIMITER.to_string(),
        ]
    } else {
        vec![format!("{key}={text}")]
    }
}

/// Port of github_env.py::append_lines.
///
/// Opens the file in append mode (creating it when missing) and writes the
/// lines joined with `\n` plus one trailing newline, as UTF-8 bytes.
///
/// NOTE: Python opens in text mode, which turns `\n` into CRLF on Windows;
/// we always write LF, matching Python's behavior on Linux CI where these
/// files are consumed (the Actions runner accepts both line endings).
pub fn append_lines(path: impl AsRef<Path>, lines: &[String]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    let mut payload = lines.join("\n");
    payload.push('\n');
    file.write_all(payload.as_bytes())
}

/// Port of github_env.py::build_run_url.
///
/// Link back to the Actions run that produced the artifacts, when in CI.
/// Missing or empty `GITHUB_REPOSITORY`/`GITHUB_RUN_ID` yields "".
///
/// NOTE quirk: a set-but-empty `GITHUB_SERVER_URL` is used verbatim - Python's
/// `os.getenv` default only applies when the variable is unset, producing a
/// server-less URL.
pub fn build_run_url() -> String {
    let repo = non_empty_env("GITHUB_REPOSITORY");
    let run_id = non_empty_env("GITHUB_RUN_ID");
    let (Some(repo), Some(run_id)) = (repo, run_id) else {
        return String::new();
    };
    let server =
        std::env::var("GITHUB_SERVER_URL").unwrap_or_else(|_| "https://github.com".to_string());
    format!("{server}/{repo}/actions/runs/{run_id}")
}

/// Python falsiness: an unset variable (None) and an empty string both count
/// as "missing".
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// Port of github_env.py::write_env.
///
/// Publishes values to GITHUB_ENV (later steps) and GITHUB_OUTPUT (later
/// jobs). With neither variable set (local/non-CI path) every pair is printed
/// to stdout as `KEY=value` and nothing is written.
///
/// The frozen Rust signature takes pre-stringified `String` values — the
/// caller decides how a `None` from the Python dict renders (contract:
/// _migration/pe-golden/write_env_contract.md). The input slice mirrors
/// Python dict insertion order and is preserved end to end.
///
/// When GITHUB_OUTPUT is set it additionally receives an `env_json` line
/// carrying every key as one compact JSON blob so a downstream job can
/// restore them all, including the per-target keys whose names come from the
/// config and therefore cannot be enumerated in workflow YAML.
pub fn write_env(values: &[(String, String)]) -> anyhow::Result<()> {
    let env_file = non_empty_env("GITHUB_ENV");
    let output_file = non_empty_env("GITHUB_OUTPUT");

    if env_file.is_none() && output_file.is_none() {
        for line in stdout_lines(values) {
            println!("{line}");
        }
        return Ok(());
    }

    let lines = build_lines(values);

    if let Some(env_file) = &env_file {
        append_lines(env_file, &lines)?;
    }

    if let Some(output_file) = output_file {
        let mut output_lines = lines;
        output_lines.extend(format_pair("env_json", Some(&env_json_blob(values))));
        append_lines(output_file, &output_lines)?;
    }

    Ok(())
}

/// Non-CI stdout rendering: Python prints `f"{key}={value}"` per pair with no
/// heredoc handling, so a newline-containing value yields two physical lines.
fn stdout_lines(values: &[(String, String)]) -> Vec<String> {
    values
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect()
}

/// CI-path line construction: every pair expanded through `format_pair`, in
/// slice order (Python: `lines.extend(format_pair(key, value))`).
fn build_lines(values: &[(String, String)]) -> Vec<String> {
    let mut lines = Vec::new();
    for (key, value) in values {
        lines.extend(format_pair(key, Some(value)));
    }
    lines
}

/// Compact, order-preserving JSON for the `env_json` output, byte-for-byte
/// equal to Python's
/// `json.dumps(values, ensure_ascii=False, separators=(",", ":"))`.
///
/// Hand-assembled from the ordered slice instead of serde_json's Map: the
/// default Map is a sorted BTreeMap and would silently reorder keys (the
/// "preserve_order" feature would fix that but also changes Map semantics for
/// every other module in the crate, so it was deliberately not enabled).
/// Per-string escaping is delegated to serde_json's string serializer, which
/// matches Python's escape table for ensure_ascii=False: short escapes for
/// \b \t \n \f \r, lowercase \u00xx for other C0 controls, raw UTF-8 else.
fn env_json_blob(values: &[(String, String)]) -> String {
    let body = values
        .iter()
        .map(|(key, value)| {
            let key_json = json_string(key);
            let value_json = json_string(value);
            format!("{key_json}:{value_json}")
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

/// One JSON string literal, escaped exactly as Python's
/// `json.dumps(..., ensure_ascii=False)` escapes a string. Serializing a
/// `&str` cannot fail.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).expect("serializing a string cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Unique temp path per call; tests clean up after themselves.
    fn temp_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "portable_builder_gh_env_{}_{nanos}_{tag}",
            std::process::id()
        ))
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn plain_value_formats_as_key_equals_value() {
        assert_eq!(format_pair("KEY", Some("value")), vec!["KEY=value"]);
        assert_eq!(
            format_pair("UPDATE_NEEDED", Some("true")),
            vec!["UPDATE_NEEDED=true"]
        );
    }

    #[test]
    fn none_and_empty_values_format_as_bare_key() {
        assert_eq!(format_pair("KEY", None), vec!["KEY="]);
        assert_eq!(format_pair("KEY", Some("")), vec!["KEY="]);
    }

    #[test]
    fn multiline_value_uses_heredoc_delimiter() {
        assert_eq!(
            format_pair("RELEASE_BODY", Some("line one\nline two")),
            vec![
                "RELEASE_BODY<<PORTABLE_BUILDER_EOF".to_string(),
                "line one\nline two".to_string(),
                "PORTABLE_BUILDER_EOF".to_string(),
            ]
        );
    }

    #[test]
    fn trailing_newline_alone_triggers_heredoc() {
        assert_eq!(
            format_pair("K", Some("v\n")),
            vec![
                "K<<PORTABLE_BUILDER_EOF".to_string(),
                "v\n".to_string(),
                "PORTABLE_BUILDER_EOF".to_string(),
            ]
        );
    }

    #[test]
    fn carriage_return_alone_stays_plain() {
        // Python only checks "\n" in text.
        assert_eq!(format_pair("K", Some("v\r")), vec!["K=v\r"]);
    }

    #[test]
    fn append_lines_creates_and_appends_with_trailing_newline() {
        let path = temp_path("append");
        let _ = std::fs::remove_file(&path);

        append_lines(&path, &["A=1".to_string(), "B=2".to_string()]).unwrap();
        assert_eq!(read(&path), "A=1\nB=2\n");

        append_lines(&path, &["C=3".to_string()]).unwrap();
        assert_eq!(read(&path), "A=1\nB=2\nC=3\n");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn append_lines_writes_utf8() {
        let path = temp_path("utf8");
        let _ = std::fs::remove_file(&path);
        append_lines(
            &path,
            &["RELEASE_TITLE=\u{6807}\u{9898} \u{503c}".to_string()],
        )
        .unwrap();
        assert_eq!(read(&path), "RELEASE_TITLE=\u{6807}\u{9898} \u{503c}\n");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn append_lines_empty_list_writes_bare_newline() {
        let path = temp_path("empty");
        let _ = std::fs::remove_file(&path);
        append_lines(&path, &[]).unwrap();
        assert_eq!(read(&path), "\n");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn append_lines_roundtrips_format_pair_output() {
        let path = temp_path("pair");
        let _ = std::fs::remove_file(&path);
        let mut lines = format_pair("UPSTREAM_VERSION", Some("147.0.1"));
        lines.extend(format_pair("RELEASE_BODY", Some("a\nb")));
        append_lines(&path, &lines).unwrap();
        assert_eq!(
            read(&path),
            "UPSTREAM_VERSION=147.0.1\nRELEASE_BODY<<PORTABLE_BUILDER_EOF\na\nb\nPORTABLE_BUILDER_EOF\n"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn build_run_url_env_fallback() {
        // Env vars are process-global; this is the only env-mutating test in
        // the crate, so run all its cases sequentially and restore state.
        let keys = ["GITHUB_REPOSITORY", "GITHUB_RUN_ID", "GITHUB_SERVER_URL"];
        let saved: Vec<(String, Option<String>)> = keys
            .iter()
            .map(|key| (key.to_string(), std::env::var(key).ok()))
            .collect();
        for key in keys {
            std::env::remove_var(key);
        }

        // Nothing set -> empty.
        assert_eq!(build_run_url(), "");

        // Only the repository -> still empty.
        std::env::set_var("GITHUB_REPOSITORY", "owner/repo");
        assert_eq!(build_run_url(), "");

        // Repository + run id -> default server https://github.com.
        std::env::set_var("GITHUB_RUN_ID", "12345");
        assert_eq!(
            build_run_url(),
            "https://github.com/owner/repo/actions/runs/12345"
        );

        // Custom server wins.
        std::env::set_var("GITHUB_SERVER_URL", "https://gh.example.com");
        assert_eq!(
            build_run_url(),
            "https://gh.example.com/owner/repo/actions/runs/12345"
        );

        // Empty repository counts as missing (Python falsy quirk).
        std::env::set_var("GITHUB_REPOSITORY", "");
        assert_eq!(build_run_url(), "");

        // Empty run id counts as missing too.
        std::env::set_var("GITHUB_REPOSITORY", "owner/repo");
        std::env::set_var("GITHUB_RUN_ID", "");
        assert_eq!(build_run_url(), "");

        // Quirk: set-but-empty server is used verbatim, no default fallback.
        std::env::set_var("GITHUB_RUN_ID", "99");
        std::env::set_var("GITHUB_SERVER_URL", "");
        assert_eq!(build_run_url(), "/owner/repo/actions/runs/99");

        // Restore the caller's environment.
        for (key, value) in saved {
            match value {
                Some(v) => std::env::set_var(&key, v),
                None => std::env::remove_var(&key),
            }
        }
    }

    #[test]
    fn env_json_blob_matches_python_sample_byte_for_byte() {
        let values = vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "x\ny".to_string()),
        ];
        // Python: json.dumps({"a": "1", "b": "x\ny"}, ensure_ascii=False,
        //                     separators=(",", ":")) — the newline is escaped.
        assert_eq!(env_json_blob(&values), "{\"a\":\"1\",\"b\":\"x\\ny\"}");
    }

    #[test]
    fn env_json_blob_preserves_insertion_order() {
        // b before a must serialize b-first. Hand-assembly guarantees it;
        // serde_json's default Map (sorted BTreeMap) would silently reorder.
        let values = vec![
            ("b".to_string(), "2".to_string()),
            ("a".to_string(), "1".to_string()),
        ];
        assert_eq!(env_json_blob(&values), "{\"b\":\"2\",\"a\":\"1\"}");
    }

    #[test]
    fn env_json_blob_escapes_like_python_json_dumps() {
        // Value chars: a " b \ c TAB d — JSON escapes \" \\ \t, nothing else.
        let values = vec![("k".to_string(), "a\"b\\c\td".to_string())];
        assert_eq!(env_json_blob(&values), "{\"k\":\"a\\\"b\\\\c\\td\"}");
    }

    #[test]
    fn env_json_blob_keeps_non_ascii_raw() {
        // ensure_ascii=False: no \uXXXX escapes for non-ASCII.
        let values = vec![("标题".to_string(), "值 147.0.1".to_string())];
        assert_eq!(env_json_blob(&values), "{\"标题\":\"值 147.0.1\"}");
    }

    #[test]
    fn env_json_blob_empty_is_empty_object() {
        assert_eq!(env_json_blob(&[]), "{}");
    }

    #[test]
    fn stdout_lines_is_plain_key_value_even_for_newlines() {
        // Python's non-CI path prints f"{key}={value}" — no heredoc on stdout.
        let values = vec![("K".to_string(), "a\nb".to_string())];
        assert_eq!(stdout_lines(&values), vec!["K=a\nb"]);
    }

    #[test]
    fn build_lines_expands_format_pair_in_order() {
        let values = vec![
            ("A".to_string(), "1".to_string()),
            ("B".to_string(), "x\ny".to_string()),
        ];
        assert_eq!(
            build_lines(&values),
            vec![
                "A=1".to_string(),
                "B<<PORTABLE_BUILDER_EOF".to_string(),
                "x\ny".to_string(),
                "PORTABLE_BUILDER_EOF".to_string(),
            ]
        );
    }

    #[test]
    fn write_env_routes_output_like_python() {
        // Env vars are process-global: run every case inside this one test so
        // parallel tests cannot race, and restore the caller's env after.
        let keys = ["GITHUB_ENV", "GITHUB_OUTPUT"];
        let saved: Vec<(String, Option<String>)> = keys
            .iter()
            .map(|key| (key.to_string(), std::env::var(key).ok()))
            .collect();
        for key in keys {
            std::env::remove_var(key);
        }

        let values = vec![
            ("b".to_string(), "x\ny".to_string()),
            ("a".to_string(), "1".to_string()),
        ];

        // Non-CI path: only prints (captured by the harness), returns Ok.
        assert!(write_env(&values).is_ok());

        // GITHUB_ENV only: heredoc for the multiline value, plain line
        // otherwise, and no env_json anywhere.
        let env_path = temp_path("we_env");
        let _ = std::fs::remove_file(&env_path);
        std::env::set_var("GITHUB_ENV", &env_path);
        write_env(&values).unwrap();
        assert_eq!(
            read(&env_path),
            "b<<PORTABLE_BUILDER_EOF\nx\ny\nPORTABLE_BUILDER_EOF\na=1\n"
        );

        // Both files: GITHUB_OUTPUT additionally gets lines + env_json, with
        // the blob preserving slice order (b first) and escaping the newline,
        // so env_json itself stays a plain KEY=value line.
        let out_path = temp_path("we_out");
        let _ = std::fs::remove_file(&out_path);
        std::env::set_var("GITHUB_OUTPUT", &out_path);
        write_env(&values).unwrap();
        assert_eq!(
            read(&env_path),
            "b<<PORTABLE_BUILDER_EOF\nx\ny\nPORTABLE_BUILDER_EOF\na=1\nb<<PORTABLE_BUILDER_EOF\nx\ny\nPORTABLE_BUILDER_EOF\na=1\n"
        );
        assert_eq!(
            read(&out_path),
            "b<<PORTABLE_BUILDER_EOF\nx\ny\nPORTABLE_BUILDER_EOF\na=1\nenv_json={\"b\":\"x\\ny\",\"a\":\"1\"}\n"
        );

        // Restore the caller's environment and clean up.
        for (key, value) in saved {
            match value {
                Some(v) => std::env::set_var(&key, v),
                None => std::env::remove_var(&key),
            }
        }
        std::fs::remove_file(&env_path).ok();
        std::fs::remove_file(&out_path).ok();
    }

    #[test]
    fn envjson_ci_golden_file_replay() {
        // Byte-level replay of the Python write_env output captured in
        // _migration/pe-golden/envjson_ci_golden.json (round 23).
        let golden_path = std::path::Path::new("_migration/pe-golden/envjson_ci_golden.json");
        if !golden_path.exists() {
            eprintln!("golden not present in this checkout; skipping");
            return;
        }
        let golden: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(golden_path).expect("read golden"))
                .unwrap();
        let expected_env = golden["github_env_bytes"].as_str().unwrap();
        let expected_out = golden["github_output_bytes"].as_str().unwrap();

        let values: Vec<(String, String)> = vec![
            ("UPDATE_NEEDED".to_string(), "true".to_string()),
            ("UPSTREAM_VERSION".to_string(), "153.1.95.102".to_string()),
            ("CREATE_NEW_RELEASE".to_string(), "false".to_string()),
            ("MINOR_UPDATE".to_string(), "false".to_string()),
            ("RELEASE_ID".to_string(), "123456".to_string()),
            (
                "RELEASE_TAG".to_string(),
                "Chrome-v153.1.95.102".to_string(),
            ),
            ("CHROME_PLUS_VERSION".to_string(), "1.18.2".to_string()),
        ];

        let dir = std::env::temp_dir().join(format!("pe-envjson-replay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let env_file = dir.join("github_env.txt");
        let out_file = dir.join("github_output.txt");
        std::fs::write(&env_file, "").unwrap();
        std::fs::write(&out_file, "").unwrap();

        std::env::set_var("GITHUB_ENV", &env_file);
        std::env::set_var("GITHUB_OUTPUT", &out_file);
        let result = std::panic::catch_unwind(|| write_env(&values).expect("write_env"));
        std::env::remove_var("GITHUB_ENV");
        std::env::remove_var("GITHUB_OUTPUT");
        result.expect("write_env failed");

        let got_env = std::fs::read_to_string(&env_file).unwrap();
        let got_out = std::fs::read_to_string(&out_file).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            got_env, expected_env,
            "GITHUB_ENV bytes differ from Python golden"
        );
        assert_eq!(
            got_out, expected_out,
            "GITHUB_OUTPUT bytes (incl. env_json blob) differ from Python golden"
        );
    }
}
