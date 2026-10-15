//! Keys consumed: url, path/installer_path, version, version_regex, file_name,
//! verify_ssl (default true), sha256, size. Exact rules (direct_script_contract.json):
//! - url XOR path required, else `direct provider requires 'url' or 'path'`,
//! - version_regex extracts group(1) from the URL when version is absent,
//! - no version at all => "0.0.0.0",
//! - file_name fallback chain: config file_name > URL last segment >
//!   "browser-installer.exe".

use anyhow::{bail, Result};
use serde_json::{json, Value};

pub fn get_package(config: &Value) -> Result<Value> {
    resolve(DirectParams {
        url: str_field(config, "url"),
        path: str_or_num_field(config, "path")
            .or_else(|| str_or_num_field(config, "installer_path")),
        version: str_field(config, "version"),
        version_regex: str_field(config, "version_regex"),
        file_name: str_field(config, "file_name"),
        verify_ssl: config
            .get("verify_ssl")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        sha256: config
            .get("sha256")
            .filter(|v| !v.is_null())
            .map(Value::to_string_value),
        size: config.get("size").and_then(Value::as_u64),
    })
}

pub struct DirectParams {
    pub url: Option<String>,
    pub path: Option<String>,
    pub version: Option<String>,
    pub version_regex: Option<String>,
    pub file_name: Option<String>,
    pub verify_ssl: bool,
    pub sha256: Option<String>,
    pub size: Option<u64>,
}

pub fn resolve(params: DirectParams) -> Result<Value> {
    let DirectParams {
        url,
        path,
        version,
        version_regex,
        file_name,
        verify_ssl,
        sha256,
        size,
    } = params;
    if url.is_none() && path.is_none() {
        bail!("direct provider requires 'url' or 'path'");
    }

    let mut version = version.filter(|v| !v.is_empty());
    if version.is_none() {
        if let (Some(pattern), Some(url)) = (version_regex.as_deref(), url.as_deref()) {
            if let Ok(re) = regex::Regex::new(pattern) {
                if let Some(caps) = re.captures(url) {
                    if let Some(m) = caps.get(1) {
                        version = Some(m.as_str().to_string());
                    }
                }
            }
        }
    }
    let version = version.unwrap_or_else(|| "0.0.0.0".to_string());

    let derived_file_name = url.as_deref().and_then(|u| {
        u.trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    });
    let file_name = file_name
        .filter(|f| !f.is_empty())
        .or(derived_file_name)
        .unwrap_or_else(|| "browser-installer.exe".to_string());

    let mut package = json!({
        "version": version,
        "file_name": file_name,
        "verify_ssl": verify_ssl,
        "sha256": sha256,
        "size": size,
    });
    if let Some(path) = path {
        package["path"] = json!(path);
    } else {
        package["url"] = json!(url.expect("url present when path absent"));
    }
    Ok(package)
}

fn str_field(config: &Value, key: &str) -> Option<String> {
    config.get(key).and_then(Value::as_str).map(str::to_string)
}

fn str_or_num_field(config: &Value, key: &str) -> Option<String> {
    match config.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

trait ToStringValue {
    fn to_string_value(&self) -> String;
}
impl ToStringValue for Value {
    fn to_string_value(&self) -> String {
        match self {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resolve_short(
        url: Option<&str>,
        path: Option<&str>,
        version: Option<&str>,
        version_regex: Option<&str>,
        file_name: Option<&str>,
    ) -> Result<Value> {
        resolve(DirectParams {
            url: url.map(str::to_string),
            path: path.map(str::to_string),
            version: version.map(str::to_string),
            version_regex: version_regex.map(str::to_string),
            file_name: file_name.map(str::to_string),
            verify_ssl: true,
            sha256: None,
            size: None,
        })
    }

    #[test]
    fn url_regex_case_from_golden() {
        // direct_case_url_regex from _migration/pe-golden/providers/direct_script_contract.json
        let pkg = resolve_short(
            Some("https://example.invalid/Vivaldi.8.2.4133.52.x64.exe"),
            None,
            None,
            Some(r"Vivaldi\.([0-9.]+)\."),
            None,
        )
        .unwrap();
        assert_eq!(pkg["version"], json!("8.2.4133.52"));
        assert_eq!(pkg["file_name"], json!("Vivaldi.8.2.4133.52.x64.exe"));
        assert_eq!(
            pkg["url"],
            json!("https://example.invalid/Vivaldi.8.2.4133.52.x64.exe")
        );
        assert_eq!(pkg["verify_ssl"], json!(true));
    }

    #[test]
    fn no_version_falls_back_to_zero_quadruple() {
        // direct_case_no_version from the golden contract.
        let pkg = resolve_short(
            Some("https://example.invalid/setup.exe"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(pkg["version"], json!("0.0.0.0"));
        assert_eq!(pkg["file_name"], json!("setup.exe"));
    }

    #[test]
    fn file_name_fallback_chain() {
        let pkg = resolve_short(
            Some("https://example.invalid/op.exe"),
            None,
            None,
            None,
            Some("custom.exe"),
        )
        .unwrap();
        assert_eq!(pkg["file_name"], json!("custom.exe"));

        let pkg = resolve_short(None, Some("C:/installers/x.exe"), None, None, None).unwrap();
        assert_eq!(pkg["file_name"], json!("browser-installer.exe"));
        assert_eq!(pkg["path"], json!("C:/installers/x.exe"));
        assert!(pkg.get("url").is_none());

        let pkg = resolve_short(Some("https://example.invalid/"), None, None, None, None).unwrap();
        assert_eq!(pkg["file_name"], json!("example.invalid"));
    }

    #[test]
    fn missing_url_and_path_is_valueerror_text() {
        let err = resolve_short(None, None, None, None, None).unwrap_err();
        assert_eq!(err.to_string(), "direct provider requires 'url' or 'path'");
    }

    #[test]
    fn installer_path_alias_and_json_entry_point() {
        let config = json!({
            "installer_path": "C:/local/setup.exe",
            "version": "1.2.3.4",
            "verify_ssl": false,
            "sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd",
            "size": 1234
        });
        let pkg = get_package(&config).unwrap();
        assert_eq!(pkg["path"], json!("C:/local/setup.exe"));
        assert_eq!(pkg["version"], json!("1.2.3.4"));
        assert_eq!(pkg["verify_ssl"], json!(false));
        assert_eq!(pkg["size"], json!(1234));
    }

    #[test]
    fn version_regex_non_matching_leaves_fallback() {
        let pkg = resolve_short(
            Some("https://example.invalid/setup.exe"),
            None,
            None,
            Some(r"Vivaldi\.([0-9.]+)\."),
            None,
        )
        .unwrap();
        assert_eq!(pkg["version"], json!("0.0.0.0"));
    }
}
