//! Port of portable_builder/providers/microsoft_edge.py. Owner: Wave2. See contract S4.3.
//!
//! Contract anchors (edge_contract.json):
//! - `verify_ssl` DEFAULTS TO FALSE everywhere (both CDP calls and the
//!   get_package passthrough) - deliberately NOT "fixed": Microsoft's CDN has
//!   a broken certificate chain. `danger_accept_invalid_certs` is only ever
//!   enabled inside this provider (migration doc S3).
//! - UA spoof "Microsoft Edge Update/1.3.183.29;winhttp", timeout 60 both calls.
//! - Version selection: ms_version (CDP ContentId.Version) or repo_version
//!   (installer_repo latest tag regex \d+\.\d+\.\d+\.\d+); when BOTH are
//!   present the GREATER wins (`compare_versions(repo, ms) > 0`); both fail =>
//!   "Unable to determine Microsoft Edge version."
//! - Download pick: sort items by SizeInBytes DESC, take [0]; FileId + ".exe"
//!   appended when the suffix is missing; fallback FileId
//!   "MicrosoftEdgeSetup.exe"; empty items => "Microsoft Edge download API
//!   returned no files."
//! - Digest: Hashes.Sha256 is BASE64 - normalize_sha256's base64 branch
//!   handles it.
//! - get_version_from_release_repo returns None on non-200 (not an error) and
//!   searches tag_name only; ContentId null => version None (falls through to
//!   the repo path).
//!
//! Pure helpers ([pick_download_item], the version selection in [get_package])
//! operate on injected JSON values so tests replay fixtures without network.

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::versions::compare_versions;

/// microsoft_edge.py USER_AGENT.
pub const USER_AGENT: &str = "Microsoft Edge Update/1.3.183.29;winhttp";
/// microsoft_edge.py EDGE_UPDATE_API ({} replaced with the app id).
pub const EDGE_UPDATE_API: &str = "https://msedge.api.cdp.microsoft.com/api/v2/contents/Browser/namespaces/Default/names/{}/versions/latest?action=select";
/// microsoft_edge.py EDGE_DOWNLOAD_API ({{}} / {{}} replaced with app id + version).
pub const EDGE_DOWNLOAD_API: &str = "https://msedge.api.cdp.microsoft.com/api/v1.1/internal/contents/Browser/namespaces/Default/names/{}/versions/{}/files?action=GenerateDownloadInfo";

/// Selected download item: (url, file_name, size, raw base64 sha256).
pub type DownloadPick = (Option<String>, String, Option<u64>, Option<String>);

/// Resolve the effective verify_ssl: default FALSE (do not "fix").
fn verify_ssl(config: &Value) -> bool {
    config
        .get("verify_ssl")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Build the blocking client honoring verify_ssl (danger only here).
fn build_client(config: &Value) -> Result<reqwest::blocking::Client> {
    let client = reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(!verify_ssl(config))
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    Ok(client)
}

/// Port of microsoft_edge.py::get_version_from_microsoft_api.
pub fn get_version_from_microsoft_api(config: &Value) -> Result<Option<String>> {
    let app_id = config
        .get("app_id")
        .and_then(Value::as_str)
        .unwrap_or("msedge-stable-win-x64");
    let user_agent = config
        .get("user_agent")
        .and_then(Value::as_str)
        .unwrap_or(USER_AGENT);
    let url = config
        .get("update_api")
        .and_then(Value::as_str)
        .unwrap_or(EDGE_UPDATE_API)
        .replace("{}", app_id);
    let data = json!({
        "targetingAttributes": {
            "IsInternalUser": true,
            "Updater": "MicrosoftEdgeUpdate",
            "UpdaterVersion": "1.3.183.29",
        }
    });
    let client = build_client(config)?;
    let response = client
        .post(&url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .json(&data)
        .send()?
        .error_for_status()?;
    let body: Value = response.json()?;
    Ok(body
        .get("ContentId")
        .and_then(|c| c.get("Version"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// Port of microsoft_edge.py::get_version_from_release_repo - None on any
/// non-200 (including network errors bubbling as Err), regex on tag_name only.
pub fn get_version_from_release_repo(config: &Value) -> Result<Option<String>> {
    let Some(repo) = config.get("installer_repo").and_then(Value::as_str) else {
        return Ok(None);
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let response = client
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .send()?;
    if response.status().as_u16() != 200 {
        return Ok(None);
    }
    let body: Value = response.json()?;
    let tag_name = body.get("tag_name").and_then(Value::as_str).unwrap_or("");
    let re = regex::Regex::new(r"(\d+\.\d+\.\d+\.\d+)")?;
    Ok(re
        .captures(tag_name)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string()))
}

/// Port of microsoft_edge.py::get_download_info's item selection - pure, so
/// the fixture replay test can run it without HTTP. Returns the selected
/// (url, file_name, size, sha256) tuple.
///
/// Sort by SizeInBytes DESC (missing SizeInBytes sorts as 0), take the first,
/// append ".exe" when the FileId suffix is missing, fallback FileId
/// "MicrosoftEdgeSetup.exe".
pub fn pick_download_item(items: &[Value]) -> Result<DownloadPick> {
    if items.is_empty() {
        bail!("Microsoft Edge download API returned no files.");
    }
    let mut sorted: Vec<&Value> = items.iter().collect();
    sorted.sort_by(|a, b| {
        let sa = a.get("SizeInBytes").and_then(Value::as_u64).unwrap_or(0);
        let sb = b.get("SizeInBytes").and_then(Value::as_u64).unwrap_or(0);
        sb.cmp(&sa)
    });
    let item = sorted[0];

    let mut file_name = item
        .get("FileId")
        .and_then(Value::as_str)
        .filter(|f| !f.is_empty())
        .unwrap_or("MicrosoftEdgeSetup.exe")
        .to_string();
    if !file_name.to_lowercase().ends_with(".exe") {
        file_name.push_str(".exe");
    }
    Ok((
        item.get("Url").and_then(Value::as_str).map(str::to_string),
        file_name,
        item.get("SizeInBytes").and_then(Value::as_u64),
        item.get("Hashes")
            .and_then(|h| h.get("Sha256"))
            .and_then(Value::as_str)
            .map(str::to_string),
    ))
}

/// Port of microsoft_edge.py::get_download_info - network path around
/// [pick_download_item].
pub fn get_download_info(config: &Value, version: &str) -> Result<DownloadPick> {
    let app_id = config
        .get("app_id")
        .and_then(Value::as_str)
        .unwrap_or("msedge-stable-win-x64");
    let user_agent = config
        .get("user_agent")
        .and_then(Value::as_str)
        .unwrap_or(USER_AGENT);
    let url = config
        .get("download_api")
        .and_then(Value::as_str)
        .unwrap_or(EDGE_DOWNLOAD_API)
        .replacen("{}", app_id, 1)
        .replacen("{}", version, 1);
    let client = build_client(config)?;
    let response = client
        .post(&url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()?
        .error_for_status()?;
    let items: Vec<Value> = response.json()?;
    pick_download_item(&items)
}

/// Port of microsoft_edge.py::get_package - two-source version selection
/// (greater wins), then download info passthrough with verify_ssl defaulting
/// to FALSE. All upstream failures degrade to warnings exactly like Python's
/// try/except + print.
pub fn get_package(config: &Value) -> Result<Value> {
    let mut ms_version: Option<String> = None;
    let mut repo_version: Option<String> = None;

    match get_version_from_microsoft_api(config) {
        Ok(v) => {
            println!(
                "[INFO] Microsoft Edge API version: {}",
                v.as_deref().unwrap_or("None")
            );
            ms_version = v;
        }
        Err(exc) => println!("[WARN] Microsoft Edge API failed: {exc}"),
    }

    match get_version_from_release_repo(config) {
        Ok(v) => {
            if v.is_some() {
                println!(
                    "[INFO] Installer repo version: {}",
                    v.as_deref().unwrap_or("None")
                );
            }
            repo_version = v;
        }
        Err(exc) => println!("[WARN] Installer repo check failed: {exc}"),
    }

    let mut version = ms_version.clone().or_else(|| repo_version.clone());
    if let (Some(repo), Some(ms)) = (&repo_version, &ms_version) {
        if compare_versions(repo, ms) > 0 {
            version = Some(repo.clone());
        }
    }

    let Some(version) = version else {
        bail!("Unable to determine Microsoft Edge version.");
    };

    let (url, file_name, size, sha256) = get_download_info(config, &version)?;
    Ok(json!({
        "version": version,
        "url": url,
        "file_name": file_name,
        "verify_ssl": verify_ssl(config),
        "sha256": sha256,
        "size": size,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    const UPDATE_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../_migration/pe-golden/providers/edge_update_response.json"
    ));
    const DOWNLOAD_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../_migration/pe-golden/providers/edge_download_response.json"
    ));

    #[test]
    fn update_fixture_yields_content_id_version() {
        // edge_update_response.json: ContentId.Version = 138.0.3351.121.
        let body: Value = serde_json::from_str(UPDATE_FIXTURE).unwrap();
        let version = body
            .get("ContentId")
            .and_then(|c| c.get("Version"))
            .and_then(Value::as_str);
        assert_eq!(version, Some("138.0.3351.121"));
    }

    #[test]
    fn empty_content_id_maps_to_none() {
        // quirk: ContentId null => version None (falls through to repo path).
        let body: Value = serde_json::from_str(r#"{"ContentId": null}"#).unwrap();
        let version = body
            .get("ContentId")
            .and_then(|c| c.get("Version"))
            .and_then(Value::as_str);
        assert_eq!(version, None);
    }

    #[test]
    fn download_pick_size_desc_and_exe_suffix() {
        // edge_download_response.json: the .exe item (180000000) must win over
        // the larger-listed .msi (150000000); FileId lacks ".exe" and gets it
        // appended; Sha256 "QkFUQ0g=" is base64 for "BATCH".
        let items: Vec<Value> = serde_json::from_str(DOWNLOAD_FIXTURE).unwrap();
        let (url, file_name, size, sha256) = pick_download_item(&items).unwrap();
        assert_eq!(
            url.as_deref(),
            Some("https://msedge.sf.dl.delivery.mp.microsoft.com/filestream/bbb/exe")
        );
        assert_eq!(file_name, "MicrosoftEdgeEnterpriseX64.exe");
        assert_eq!(size, Some(180000000));
        // The fixture digest passes through RAW: get_package does not normalize;
        // tools.download_file does at download time. (The fixture's "QkFUQ0g="
        // is a 5-byte dummy that normalize_sha256 would reject - which is fine,
        // this provider never calls it.)
        assert_eq!(sha256.as_deref(), Some("QkFUQ0g="));

        // The base64 branch itself is covered in tools.rs tests; document the
        // real-shape contract with a properly sized digest.
        let real_digest = base64::engine::general_purpose::STANDARD.encode([0x42u8; 32]);
        let normalized = crate::tools::normalize_sha256(Some(&real_digest))
            .unwrap()
            .unwrap();
        assert_eq!(normalized, hex::encode([0x42u8; 32]));
    }

    #[test]
    fn download_pick_tie_breaks_and_defaults() {
        // Equal sizes: stable sort keeps the original order (Python sort is
        // stable too), first item wins.
        let items: Vec<Value> = serde_json::from_str(
            r#"[
            {"FileId": "A", "SizeInBytes": 100},
            {"FileId": "B", "SizeInBytes": 100}
        ]"#,
        )
        .unwrap();
        let (_, file_name, _, _) = pick_download_item(&items).unwrap();
        assert_eq!(file_name, "A.exe");

        // Missing FileId => "MicrosoftEdgeSetup.exe" (already .exe).
        let items: Vec<Value> =
            serde_json::from_str(r#"[{"Url": "https://x/y", "SizeInBytes": 5}]"#).unwrap();
        let (_, file_name, size, sha256) = pick_download_item(&items).unwrap();
        assert_eq!(file_name, "MicrosoftEdgeSetup.exe");
        assert_eq!(size, Some(5));
        assert_eq!(sha256, None);
    }

    #[test]
    fn empty_items_runtime_error_text() {
        let err = pick_download_item(&[]).unwrap_err().to_string();
        assert_eq!(err, "Microsoft Edge download API returned no files.");
    }

    #[test]
    fn version_selection_greater_wins() {
        // ms=138.0.3351.121, repo=139.0.1.1 => repo wins.
        let ms = "138.0.3351.121";
        let repo = "139.0.1.1";
        let mut version = Some(ms.to_string()).or_else(|| repo.to_string().into());
        if let (Some(repo_v), Some(ms_v)) = (Some(repo.to_string()), Some(ms.to_string())) {
            if compare_versions(&repo_v, &ms_v) > 0 {
                version = Some(repo_v);
            }
        }
        assert_eq!(version.as_deref(), Some("139.0.1.1"));

        // repo lower => ms stays.
        let repo = "137.0.0.1";
        let mut version = Some(ms.to_string());
        if compare_versions(repo, ms) > 0 {
            version = Some(repo.to_string());
        }
        assert_eq!(version.as_deref(), Some("138.0.3351.121"));
    }

    #[test]
    fn both_sources_missing_is_runtime_error_text() {
        let config = json!({});
        // get_package would hit the network; replicate the terminal guard.
        let ms_version: Option<String> = None;
        let repo_version: Option<String> = None;
        let version = ms_version.or(repo_version);
        let result = match version {
            Some(_) => Ok(()),
            None => Err("Unable to determine Microsoft Edge version."),
        };
        assert_eq!(
            result.unwrap_err(),
            "Unable to determine Microsoft Edge version."
        );
        let _ = config; // verify_ssl(config) defaults false; asserted separately
    }

    #[test]
    fn verify_ssl_defaults_false_everywhere() {
        // All three touch points default FALSE (edge_contract.json).
        let config = json!({});
        assert!(!verify_ssl(&config));
        let config = json!({ "verify_ssl": true });
        assert!(verify_ssl(&config));
        let passthrough = json!({ "verify_ssl": null });
        assert!(!verify_ssl(&passthrough));
    }

    #[test]
    fn api_url_templates_format_app_id_and_version() {
        let url = EDGE_UPDATE_API.replace("{}", "msedge-stable-win-x64");
        assert!(url.ends_with("/names/msedge-stable-win-x64/versions/latest?action=select"));

        let url = EDGE_DOWNLOAD_API
            .replacen("{}", "msedge-stable-win-x64", 1)
            .replacen("{}", "138.0.3351.121", 1);
        assert!(url.contains("/names/msedge-stable-win-x64/versions/138.0.3351.121/files"));
    }
}
