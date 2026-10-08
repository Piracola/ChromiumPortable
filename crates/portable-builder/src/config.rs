//! Port of portable_builder/config.py - JSON/TOML config loading + serde Target schema. Owner: Wave1-B.
//!
//! Contract of record: docs/MIGRATION_RUST_TAURI.md §4.1.
//!
//! Divergences from Python (deliberate, per §11):
//! - YAML support is dropped; \`.yml\` / \`.yaml\` now hit the unsupported-format error.
//!
//! `get_target` injection semantics (ported 1:1 from config.py:33-34):
//! `target = dict(targets[target_name])` **copies** the target mapping out of the
//! loaded config; `target.setdefault("target", target_name)` then injects the
//! queried name into that **copy** only. The loaded `config` value is never
//! mutated. If the target already carries a `"target"` key, its own value is
//! kept (setdefault semantics = entry().or_insert_with()).
//!
//! Key → default table (every `target.get(key, default)` chain in builder.py /
//! verify.py / tools.py / multi.py / release.py; the table IS the documentation
//! required by §4.1):
//!
//! | key                     | Python default                                  | Rust field type / default fn              |
//! |-------------------------|-------------------------------------------------|--------------------------------------------|
//! | target                  | `""` (builder.py:36)                             | `String = ""`                        |
//! | name                    | chain `name → display_name → ""`                  | `Option<String> = None`              |
//! | display_name            | chain `display_name → name → ""`                  | `Option<String> = None`              |
//! | product                 | missing key is falsy in Python                  | `bool = false`                       |
//! | output_dir              | chain `output_dir → name → "Browser"`            | `Option<String> = None`              |
//! | exe_name                | `target.get("exe_name")` (builder.py:308)        | `Option<String> = None`              |
//! | architecture            | `"x64"` (builder.py:43,107,256)                  | `String = "x64"`                     |
//! | layout                  | `"move_version_root"` (builder.py:157)           | `String = "move_version_root"`       |
//! | inner_archive           | `None` (builder.py:125)                          | `Option<String> = None`              |
//! | version_root            | `"Chrome-bin"` (builder.py:132)                  | `String = "Chrome-bin"`              |
//! | provider                | `{}` (builder.py:54)                             | `Option<Value> = None`               |
//! | chrome_plus_dir         | `"chrome++"` (builder.py:207,268)                | `String = "chrome++"`                |
//! | ini_name                | `"chrome++.ini"` (builder.py:208)                | `String = "chrome++.ini"`            |
//! | ini_override_name       | `"chrome++.override.ini"` (builder.py:209)       | `String = "chrome++.override.ini"`   |
//! | version_dll_name        | `f"version-{arch}.dll"` (builder.py:257)         | `Option<String> = None` (arch-derived)|
//! | setdll_name             | `f"setdll-{arch}.exe"` (builder.py:258)          | `Option<String> = None` (arch-derived)|
//! | version_dll_location    | `"app_root"` (builder.py:279)                    | `String = "app_root"`                |
//! | ini_location            | `"app_root"` (builder.py:296)                    | `String = "app_root"`                |
//! | allow_7z_download       | `not auto_layout` (builder.py:99)                | `Option<bool> = None` (computed)     |
//! | allow_7z_system_install | `not auto_layout` (builder.py:100)               | `Option<bool> = None` (computed)     |
//! | verify_architecture     | `True` (builder.py:322)                          | `bool = true`                        |
//! | forbidden_file_names    | `[]` (tools.py:370)                              | `Vec<String> = []`                   |
//! | smoke_data_dir          | `"Data"` (verify.py:132)                         | `String = "Data"`                    |
//! | smoke_timeout           | `120` for --version probe, `180` for the      | `Option<u64> = None` (split default) |
//! |                         | GUI smoke run (verify.py:145 vs :148)           |                                            |
//! | smoke_data_timeout      | `60` (verify.py:152)                             | `u64 = 60`                           |
//! | smoke_args              | `DEFAULT_SMOKE_ARGS` (verify.py:147)             | `Option<Vec<String>> = None`         |
//! | start_script            | `"开始.bat"` (builder.py:372)                     | `String = "开始.bat"`                 |
//! | archive_name            | `"{display_name}_Portable_{version}_{date}.7z"`| `Option<String> = None`            |
//! |                         | (builder.py:479)                                |                                            |
//! | release                 | `{}` (multi.py:153, release.py:100)              | `Option<Value> = None`               |
//! | env_prefix              | `or env_name(target_name)` (multi.py:48)         | `Option<String> = None`              |
//! | remove_setdll           | `True` (builder.py:356)                          | `bool = true`                        |
//!
//! Fields typed `Option<_>` are exactly those whose Python default is computed
//! downstream (from other keys or module constants): the port stores the
//! configured value verbatim and lets the consumer apply the fallback, which
//! preserves the distinction between "absent" and "explicitly set" that the
//! Python `.get(key, default)` chains rely on.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Supported config formats after the Rust port (YAML dropped, §11).
pub const SUPPORTED_FORMATS: &str = ".json, .toml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file '{path}': {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse '{path}' as {format}: {detail}")]
    Parse {
        path: String,
        format: &'static str,
        detail: String,
    },
    #[error("Unsupported config format: {suffix} in '{path}' (supported: {supported}; YAML support was dropped in the Rust port)")]
    UnsupportedFormat {
        path: String,
        suffix: String,
        supported: &'static str,
    },
    /// Display string is byte-identical to the Python KeyError message
    /// (config.py:32): `Target 'nope' not found. Available targets: chrome_stable`.
    #[error("Target '{name}' not found. Available targets: {available}")]
    UnknownTarget { name: String, available: String },
    #[error("target '{name}' does not match the Target schema: {detail}")]
    Schema { name: String, detail: String },
}

/// Port of config.py:load_config. JSON and TOML only.
pub fn load_config(path: impl AsRef<Path>) -> Result<Value, ConfigError> {
    let path = path.as_ref();
    let path_str = path.to_string_lossy().into_owned();
    // Python: suffix = config_path.suffix.lower(); the error message keeps the original suffix.
    let suffix = path
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();

    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path_str.clone(),
        source,
    })?;

    match suffix.to_lowercase().as_str() {
        ".json" => serde_json::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path_str,
            format: "JSON",
            detail: source.to_string(),
        }),
        ".toml" => toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path_str,
            format: "TOML",
            detail: source.to_string(),
        }),
        // .yml / .yaml and everything else: the Python port raised ValueError
        // before ever attempting YAML import; the Rust port has no YAML path at all.
        _ => Err(ConfigError::UnsupportedFormat {
            path: path_str,
            suffix,
            supported: SUPPORTED_FORMATS,
        }),
    }
}

/// Port of config.py:get_target, including the injection quirk: the returned
/// mapping is a **copy** with a `"target"` key injected via setdefault
/// semantics; the loaded `config` is left untouched (see module docs).
pub fn get_target(config: &Value, target_name: &str) -> Result<Value, ConfigError> {
    // Python: targets = config.get("targets", {}) - a missing/empty "targets" key
    // is an empty mapping, and "(none)" is reported when it holds no targets.
    let targets = config.get("targets").and_then(Value::as_object);

    let source = match targets.and_then(|t| t.get(target_name)) {
        Some(source) => source,
        None => {
            let mut available: Vec<&str> = targets
                .map(|t| t.keys().map(String::as_str).collect())
                .unwrap_or_default();
            // Python sorted() over str keys == UTF-8 byte order == Rust str Ord.
            available.sort_unstable();
            let available = if available.is_empty() {
                "(none)".to_string()
            } else {
                available.join(", ")
            };
            return Err(ConfigError::UnknownTarget {
                name: target_name.to_string(),
                available,
            });
        }
    };

    // Python: target = dict(targets[target_name]) - a copy, never a reference.
    let mut target = source.clone();
    if let Value::Object(map) = &mut target {
        // Python: target.setdefault("target", target_name) - keep an existing
        // value, inject the queried name only when the key is absent.
        map.entry("target".to_string())
            .or_insert_with(|| Value::String(target_name.to_string()));
    }
    Ok(target)
}

/// serde Target schema: every key consumed downstream, with defaults exactly as
/// in the key → default table in the module docs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    /// Injected by get_target; builder.py:36 uses `target.get("target", "")`.
    #[serde(default = "default_empty_string")]
    pub target: String,
    /// builder.py:37 chain `name → display_name → ""`.
    #[serde(default)]
    pub name: Option<String>,
    /// builder.py:38 chain `display_name → name → ""`.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Product targets (Chrome / Edge / Helium) set this explicitly; absent = falsy.
    #[serde(default = "default_false")]
    pub product: bool,
    /// builder.py:39,155,363,395 chain `output_dir → name → "Browser"`.
    #[serde(default)]
    pub output_dir: Option<String>,
    /// builder.py:308 / verify.py:66 - no default; auto layout derives it from the PE.
    #[serde(default)]
    pub exe_name: Option<String>,
    /// builder.py:43,107,256 / verify.py:58 - default `"x64"`.
    #[serde(default = "default_architecture")]
    pub architecture: String,
    /// builder.py:66,75,102,157 / verify.py:57 - default `"move_version_root"`.
    #[serde(default = "default_layout")]
    pub layout: String,
    /// builder.py:125 - Edge-style inner archive name; absent for plain layouts.
    #[serde(default)]
    pub inner_archive: Option<String>,
    /// builder.py:132 - default `"Chrome-bin"`.
    #[serde(default = "default_version_root")]
    pub version_root: String,
    /// builder.py:54 - `target.get("provider", {})`; typed downstream by providers.
    #[serde(default)]
    pub provider: Option<Value>,
    /// builder.py:207,268 - default `"chrome++"`.
    #[serde(default = "default_chrome_plus_dir")]
    pub chrome_plus_dir: String,
    /// builder.py:208 - default `"chrome++.ini"`.
    #[serde(default = "default_ini_name")]
    pub ini_name: String,
    /// builder.py:209 - default `"chrome++.override.ini"`.
    #[serde(default = "default_ini_override_name")]
    pub ini_override_name: String,
    /// builder.py:257 - default `f"version-{arch}.dll"` depends on architecture,
    /// so `None` means "compute from `architecture`".
    #[serde(default)]
    pub version_dll_name: Option<String>,
    /// builder.py:258 - default `f"setdll-{arch}.exe"` depends on architecture.
    #[serde(default)]
    pub setdll_name: Option<String>,
    /// builder.py:279 - default `"app_root"`.
    #[serde(default = "default_version_dll_location")]
    pub version_dll_location: String,
    /// builder.py:296 - default `"app_root"`.
    #[serde(default = "default_ini_location")]
    pub ini_location: String,
    /// builder.py:99 / verify.py:186 - default is `not auto_layout`, computed downstream.
    #[serde(default)]
    pub allow_7z_download: Option<bool>,
    /// builder.py:100 / verify.py:187 - default is `not auto_layout`, computed downstream.
    #[serde(default)]
    pub allow_7z_system_install: Option<bool>,
    /// builder.py:322 - default `True`.
    #[serde(default = "default_true")]
    pub verify_architecture: bool,
    /// tools.py:370 - default `[]`.
    #[serde(default)]
    pub forbidden_file_names: Vec<String>,
    /// verify.py:132 - default `"Data"`.
    #[serde(default = "default_smoke_data_dir")]
    pub smoke_data_dir: String,
    /// verify.py:145 uses 120 for the `--version` probe and verify.py:148 uses 180
    /// for the GUI smoke run; `None` keeps both split defaults available.
    #[serde(default)]
    pub smoke_timeout: Option<u64>,
    /// verify.py:152 - default `60`.
    #[serde(default = "default_smoke_data_timeout")]
    pub smoke_data_timeout: u64,
    /// verify.py:147 - default is `DEFAULT_SMOKE_ARGS` (owned by verify).
    #[serde(default)]
    pub smoke_args: Option<Vec<String>>,
    /// builder.py:372 - default `"开始.bat"`.
    #[serde(default = "default_start_script")]
    pub start_script: String,
    /// builder.py:479 - default `"{display_name}_Portable_{version}_{date}.7z"`.
    #[serde(default)]
    pub archive_name: Option<String>,
    /// multi.py:153 / release.py:100 - `target.get("release", {})`.
    #[serde(default)]
    pub release: Option<Value>,
    /// multi.py:48,199,211,247,285,331 - `target.get("env_prefix") or env_name(target_name)`.
    #[serde(default)]
    pub env_prefix: Option<String>,
    /// builder.py:356 - default `True` (present in builder.py, absent from catalog).
    #[serde(default = "default_true")]
    pub remove_setdll: bool,
}

fn default_empty_string() -> String {
    String::new()
}
fn default_false() -> bool {
    false
}
fn default_true() -> bool {
    true
}
fn default_architecture() -> String {
    "x64".to_string()
}
fn default_layout() -> String {
    "move_version_root".to_string()
}
fn default_version_root() -> String {
    "Chrome-bin".to_string()
}
fn default_chrome_plus_dir() -> String {
    "chrome++".to_string()
}
fn default_ini_name() -> String {
    "chrome++.ini".to_string()
}
fn default_ini_override_name() -> String {
    "chrome++.override.ini".to_string()
}
fn default_version_dll_location() -> String {
    "app_root".to_string()
}
fn default_ini_location() -> String {
    "app_root".to_string()
}
fn default_smoke_data_dir() -> String {
    "Data".to_string()
}
fn default_smoke_data_timeout() -> u64 {
    60
}
fn default_start_script() -> String {
    "开始.bat".to_string()
}

impl Target {
    /// Convenience for downstream waves: `get_target` + `Target::deserialize`
    /// with the schema error wrapped as `ConfigError::Schema`.
    pub fn from_config(config: &Value, target_name: &str) -> Result<Target, ConfigError> {
        let value = get_target(config, target_name)?;
        serde_json::from_value(value).map_err(|source| ConfigError::Schema {
            name: target_name.to_string(),
            detail: source.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Isolated temp dir keyed by pid + test-unique file name.
    fn temp_file(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pe-config-tests-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        let path = dir.join(name);
        std::fs::write(&path, content).expect("write temp test file");
        path
    }

    const CHROME_STABLE_TOML: &str = r#"
[targets.chrome_stable]
architecture = "x64"
display_name = "Chrome++"
layout = "move_version_root"
name = "Chrome"
"#;

    /// Load _migration/pe-golden/config_i18n_reference.json - the real chrome /
    /// edge / helium target dicts, used as the golden for typed deserialization.
    fn i18n_golden() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_migration/pe-golden/config_i18n_reference.json");
        let text = std::fs::read_to_string(&path).expect("read config_i18n_reference.json");
        serde_json::from_str(&text).expect("parse config_i18n_reference.json")
    }

    #[test]
    fn load_json_and_get_target_matches_golden() {
        let path = temp_file(
            "load.json",
            r#"{"targets": {"chrome_stable": {"name": "Chrome", "display_name": "Chrome++", "architecture": "x64", "layout": "move_version_root"}}}"#,
        );
        let config = load_config(&path).expect("json config loads");
        // Golden: _migration/pe-golden/config_toml_errors_golden.json toml_get_target.
        let target = get_target(&config, "chrome_stable").expect("target found");
        assert_eq!(
            target,
            json!({
                "architecture": "x64",
                "display_name": "Chrome++",
                "layout": "move_version_root",
                "name": "Chrome",
                "target": "chrome_stable"
            })
        );
    }

    #[test]
    fn load_toml_matches_golden() {
        let path = temp_file("load.toml", CHROME_STABLE_TOML);
        let config = load_config(&path).expect("toml config loads");
        // Golden: config_toml_errors_golden.json toml_load.
        assert_eq!(
            config,
            json!({
                "targets": {
                    "chrome_stable": {
                        "architecture": "x64",
                        "display_name": "Chrome++",
                        "layout": "move_version_root",
                        "name": "Chrome"
                    }
                }
            })
        );
    }

    #[test]
    fn unsupported_suffix_error_names_file_and_formats() {
        let path = temp_file("config.txt", "not a config");
        let err = load_config(&path).expect_err("bad suffix must fail");
        let message = err.to_string();
        // Python: "Unsupported config format: {suffix}" - extended with the file
        // name and the supported list per the Wave1-B1 task contract.
        assert!(
            message.contains("Unsupported config format: .txt"),
            "{message}"
        );
        assert!(
            message.contains(&format!("'{}'", path.display())),
            "{message}"
        );
        assert!(message.contains(".json, .toml"), "{message}");
    }

    #[test]
    fn yaml_is_dropped_in_the_rust_port() {
        let path = temp_file("config.yaml", "targets: {}");
        let err = load_config(&path).expect_err("yaml must fail now");
        assert!(err.to_string().contains("Unsupported config format: .yaml"));
    }

    #[test]
    fn unknown_target_error_text_is_exact() {
        let config = load_config(temp_file("unknown.toml", CHROME_STABLE_TOML)).unwrap();
        let err = get_target(&config, "nope").expect_err("unknown target");
        // Golden: config_toml_errors_golden.json errors.unknown_target, minus the
        // Python exception-type wrapper: KeyError("...") -> "...".
        assert_eq!(
            err.to_string(),
            "Target 'nope' not found. Available targets: chrome_stable"
        );
    }

    #[test]
    fn unknown_target_with_no_targets_reports_none() {
        let config = json!({});
        let err = get_target(&config, "nope").expect_err("no targets at all");
        // Python: ", ".join(sorted(targets)) or "(none)".
        assert_eq!(
            err.to_string(),
            "Target 'nope' not found. Available targets: (none)"
        );
    }

    #[test]
    fn injection_quirk_copies_and_does_not_mutate_config() {
        // A target that already carries a "target" key keeps its own value
        // (setdefault semantics).
        let config = json!({
            "targets": {
                "weird": { "name": "Weird", "target": "something_else" }
            }
        });
        let target = get_target(&config, "weird").expect("target found");
        assert_eq!(target["target"], json!("something_else"));

        // The loaded config itself is never mutated: no "target" key
        // appears on the stored mapping after get_target runs.
        let config = load_config(temp_file("quirk.toml", CHROME_STABLE_TOML)).unwrap();
        let _ = get_target(&config, "chrome_stable").unwrap();
        assert!(config["targets"]["chrome_stable"].get("target").is_none());
    }

    #[test]
    fn defaults_table_matches_python_get_chains() {
        // Minimal target: only a name; every other field must land on the
        // Python .get() chain default from the key → default table.
        let minimal = json!({ "name": "Minimal" });
        let target: Target = serde_json::from_value(minimal).expect("schema");
        assert_eq!(target.target, "");
        assert_eq!(target.architecture, "x64");
        assert_eq!(target.layout, "move_version_root");
        assert_eq!(target.version_root, "Chrome-bin");
        assert_eq!(target.chrome_plus_dir, "chrome++");
        assert_eq!(target.ini_name, "chrome++.ini");
        assert_eq!(target.ini_override_name, "chrome++.override.ini");
        assert_eq!(target.version_dll_location, "app_root");
        assert_eq!(target.ini_location, "app_root");
        assert_eq!(target.smoke_data_dir, "Data");
        assert_eq!(target.smoke_data_timeout, 60);
        assert_eq!(target.start_script, "开始.bat");
        assert!(!target.product);
        assert!(target.verify_architecture);
        assert!(target.remove_setdll);
        assert!(target.forbidden_file_names.is_empty());
        // Computed-downstream defaults stay Option/None.
        assert_eq!(target.version_dll_name, None);
        assert_eq!(target.setdll_name, None);
        assert_eq!(target.allow_7z_download, None);
        assert_eq!(target.allow_7z_system_install, None);
        assert_eq!(target.smoke_timeout, None);
        assert_eq!(target.smoke_args, None);
        assert_eq!(target.output_dir, None);
        assert_eq!(target.archive_name, None);
        assert_eq!(target.env_prefix, None);
    }

    #[test]
    fn product_boolean_round_trip_vs_golden() {
        let golden = i18n_golden();
        // Golden target_chrome_stable: product == true.
        let chrome: Target =
            serde_json::from_value(golden["target_chrome_stable"].clone()).expect("chrome schema");
        assert!(chrome.product);
        assert_eq!(chrome.name.as_deref(), Some("Chrome"));
        assert_eq!(chrome.output_dir.as_deref(), Some("Chrome"));
        assert_eq!(chrome.exe_name.as_deref(), Some("chrome.exe"));
        assert_eq!(
            chrome.archive_name.as_deref(),
            Some("Chrome++_stable_{version}_{date}.7z")
        );
        // Serialize back: the boolean survives the round trip.
        let round = serde_json::to_value(&chrome).expect("serialize");
        assert_eq!(round["product"], json!(true));

        // catalog_reference.json full_brave_stable: product == false (Unofficial).
        let catalog_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_migration/pe-golden/catalog_reference.json");
        let catalog: Value = serde_json::from_str(
            &std::fs::read_to_string(&catalog_path).expect("read catalog_reference.json"),
        )
        .expect("parse catalog_reference.json");
        let brave: Target =
            serde_json::from_value(catalog["full_brave_stable"].clone()).expect("brave schema");
        assert!(!brave.product);
        assert_eq!(brave.output_dir.as_deref(), Some("Brave Browser"));
    }

    #[test]
    fn typed_get_target_reads_edge_golden_dict() {
        let golden = i18n_golden();
        let config = json!({ "targets": { "edge_stable": golden["target_edge_stable"].clone() } });
        let edge = Target::from_config(&config, "edge_stable").expect("edge schema");
        assert_eq!(edge.target, "edge_stable"); // injected by get_target
        assert_eq!(edge.inner_archive.as_deref(), Some("MSEDGE.7Z"));
        assert_eq!(edge.layout, "move_version_dir");
        assert_eq!(edge.version_dll_location, "version_dir");

        // The "target" key was injected into the copy, not the source dict.
        assert!(config["targets"]["edge_stable"].get("target").is_none());
    }
}
