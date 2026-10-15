//! The XML POST body, channel appid/ap table and the `nextversion=""` field
//! are frozen verbatim - the Omaha server is sensitive to those exact strings
//! (migration doc S4.3).
//!
//! Exact error texts (omaha_expected.json quirk_notes):
//! - "No manifest found in Google Omaha response."
//! - "No package found in Google Omaha response."
//! - "No download URL found in Google Omaha response."
//! - channel unknown => `Google Omaha channel '<ch>' is not configured.`

use anyhow::{bail, Result};
use serde_json::{json, Value};

pub const DEFAULT_UPDATE_URL: &str = "https://tools.google.com/service/update2";

/// CHANNELS attribute strings are frozen byte-for-byte (Omaha server contract).
pub const CHANNELS: [(&str, &str, &str); 4] = [
    (
        "win_stable_x64",
        "platform=\"win\" version=\"10.0\" sp=\"\" arch=\"x64\"",
        "appid=\"{8A69D345-D564-463C-AFF1-A69D9E530F96}\" version=\"\" nextversion=\"\" lang=\"en\" brand=\"\" installage=\"-1\" installdate=\"-1\" iid=\"{11111111-1111-1111-1111-111111111111}\"",
    ),
    (
        "win_beta_x64",
        "platform=\"win\" version=\"10.0\" arch=\"x64\"",
        "appid=\"{8A69D345-D564-463C-AFF1-A69D9E530F96}\" ap=\"x64-beta-multi-chrome\"",
    ),
    (
        "win_dev_x64",
        "platform=\"win\" version=\"10.0\" arch=\"x64\"",
        "appid=\"{8A69D345-D564-463C-AFF1-A69D9E530F96}\" ap=\"x64-dev-multi-chrome\"",
    ),
    (
        "win_canary_x64",
        "platform=\"win\" version=\"10.0\" arch=\"x64\"",
        "appid=\"{4EA16AC7-FD5A-47C3-875B-DBF4A2008C20}\" ap=\"x64-canary\"",
    ),
];

/// The request template is frozen byte-for-byte (Omaha server contract).
pub fn build_request_body(os_xml: &str, app_xml: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<request protocol="3.0" updater="Omaha" updaterversion="1.3.36.372" shell_version="1.3.36.352" ismachine="0" sessionid="{{11111111-1111-1111-1111-111111111111}}" installsource="taggedmi" requestid="{{11111111-1111-1111-1111-111111111111}}" dedup="cr" domainjoined="0">
  <hw physmemory="16" sse="1" sse2="1" sse3="1" ssse3="1" sse41="1" sse42="1" avx="1"/>
  <os {os_xml}/>
  <app {app_xml}>
    <updatecheck/>
    <data name="install" index="empty"/>
  </app>
</request>"#
    )
}

pub fn post_update(update_url: &str, os_xml: &str, app_xml: &str) -> Result<String> {
    let body = build_request_body(os_xml, app_xml);
    let client = reqwest::blocking::Client::builder()
        .user_agent(crate::release::DEFAULT_USER_AGENT)
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let response = client
        .post(update_url)
        .header(reqwest::header::CONTENT_TYPE, "text/xml")
        .body(body)
        .send()?
        .error_for_status()?;
    Ok(response.text()?)
}

fn find_element(text: &str, tag: &str) -> Option<XmlElem> {
    let bytes = text.as_bytes();
    let mut pos = 0usize;
    while let Some(found) = text[pos..].find(&format!("<{tag}")) {
        let start = pos + found;
        let after = bytes.get(start + 1 + tag.len()).copied();
        if !matches!(
            after,
            Some(b' ') | Some(b'>') | Some(b'/') | Some(b'\t') | Some(b'\r') | Some(b'\n')
        ) {
            pos = start + 1;
            continue;
        }
        let end_rel = text[start..].find('>')?;
        let end = start + end_rel;
        let open = &text[start..=end];
        let self_closing = open.trim_end().ends_with("/>");
        let attrs = parse_attrs(open);
        if !self_closing {
            let close = format!("</{tag}>");
            text[end + 1..].find(&close)?;
        }
        return Some(XmlElem { attrs });
    }
    None
}

fn find_elements(text: &str, tag: &str) -> Vec<XmlElem> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(found) = text[pos..].find(&format!("<{tag}")) {
        let start = pos + found;
        let after = text.as_bytes().get(start + 1 + tag.len()).copied();
        if !matches!(
            after,
            Some(b' ') | Some(b'>') | Some(b'/') | Some(b'\t') | Some(b'\r') | Some(b'\n')
        ) {
            pos = start + 1;
            continue;
        }
        let Some(end_rel) = text[start..].find('>') else {
            break;
        };
        let end = start + end_rel;
        let open = &text[start..=end];
        let attrs = parse_attrs(open);
        out.push(XmlElem { attrs });
        pos = end + 1;
    }
    out
}

struct XmlElem {
    attrs: Vec<(String, String)>,
}
impl XmlElem {
    fn get(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn parse_attrs(open_tag: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let bytes = open_tag.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i] != b'<' {
        i += 1;
    }
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    while i < bytes.len()
        && bytes[i] != b'>'
        && bytes[i].is_ascii_alphanumeric()
            | (bytes[i] == b'_')
            | (bytes[i] == b'-')
            | (bytes[i] == b':')
    {
        i += 1;
    }
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'>' || bytes[i] == b'/' {
            break;
        }
        let name_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let name = open_tag[name_start..i].to_string();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || (bytes[i] != b'"' && bytes[i] != b'\'') {
            continue;
        }
        let quote = bytes[i];
        i += 1;
        let value_start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        let value = open_tag[value_start..i.min(open_tag.len())].to_string();
        if i < bytes.len() {
            i += 1;
        }
        attrs.push((name, xml_unescape(&value)));
    }
    attrs
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[derive(Debug)]
pub struct OmahaDecoded {
    pub version: Option<String>,
    pub urls: Vec<String>,
    pub file_name: Option<String>,
    pub sha256: Option<String>,
    pub size: Option<u64>,
}

pub fn decode_response(text: &str) -> Result<OmahaDecoded> {
    let manifest = find_element(text, "manifest")
        .ok_or_else(|| anyhow::anyhow!("No manifest found in Google Omaha response."))?;
    let package = find_element(text, "package")
        .ok_or_else(|| anyhow::anyhow!("No package found in Google Omaha response."))?;

    let package_name = package.get("name");
    let mut urls = Vec::new();
    for node in find_elements(text, "url") {
        if let Some(codebase) = node.get("codebase") {
            urls.push(format!("{}{}", codebase, package_name.unwrap_or_default()));
        }
    }
    if urls.is_empty() {
        bail!("No download URL found in Google Omaha response.");
    }

    let size = package
        .get("size")
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .and_then(|s| s.parse::<u64>().ok());

    Ok(OmahaDecoded {
        version: manifest.get("version").map(str::to_string),
        urls,
        file_name: package_name.map(str::to_string),
        sha256: package.get("hash_sha256").map(str::to_string),
        size,
    })
}

pub fn get_package(config: &Value) -> Result<Value> {
    let channel = config
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or("win_stable_x64");
    let mut os_attr: Option<String> = None;
    let mut app_attr: Option<String> = None;
    for (name, os, app) in CHANNELS {
        if name == channel {
            os_attr = Some(os.to_string());
            app_attr = Some(app.to_string());
            break;
        }
    }
    if let Some(request) = config.get("request") {
        if let Some(os) = request.get("os").and_then(Value::as_str) {
            os_attr = Some(os.to_string());
        }
        if let Some(app) = request.get("app").and_then(Value::as_str) {
            app_attr = Some(app.to_string());
        }
    }

    let (Some(os_attr), Some(app_attr)) = (os_attr, app_attr) else {
        bail!("Google Omaha channel '{channel}' is not configured.");
    };
    if os_attr.is_empty() || app_attr.is_empty() {
        bail!("Google Omaha channel '{channel}' is not configured.");
    }

    let update_url = config
        .get("update_url")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_UPDATE_URL);
    let data = decode_response(&post_update(update_url, &os_attr, &app_attr)?)?;

    let preferred_hosts: Vec<String> = config
        .get("preferred_hosts")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| vec!["https://dl.google.com".to_string()]);

    let mut download_url = data
        .urls
        .first()
        .cloned()
        .expect("decode_response guarantees at least one URL");
    for candidate in &data.urls {
        if preferred_hosts
            .iter()
            .any(|host| candidate.starts_with(host))
        {
            download_url = candidate.clone();
            break;
        }
    }

    Ok(json!({
        "version": data.version,
        "url": download_url,
        "file_name": data.file_name.unwrap_or_else(|| "chrome.7z.exe".to_string()),
        "verify_ssl": config.get("verify_ssl").and_then(Value::as_bool).unwrap_or(true),
        "sha256": data.sha256,
        "size": data.size,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XML: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../_migration/pe-golden/providers/omaha_response_sample.xml"
    ));

    #[test]
    fn decode_matches_omaha_expected_json() {
        // Golden: _migration/pe-golden/providers/omaha_expected.json
        let decoded = decode_response(SAMPLE_XML).unwrap();
        assert_eq!(decoded.version.as_deref(), Some("153.6798.95.102"));
        assert_eq!(
            decoded.urls,
            vec![
                "https://dl.google.com/release2/chrome/83.0.4103.116_chrome_installer.exe",
                "http://cache.pack.google.com/edgedl/chrome/83.0.4103.116_chrome_installer.exe",
            ]
        );
        assert_eq!(
            decoded.file_name.as_deref(),
            Some("83.0.4103.116_chrome_installer.exe")
        );
        assert_eq!(
            decoded.sha256.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd")
        );
        assert_eq!(decoded.size, Some(98765432));
    }

    #[test]
    fn preferred_host_prefix_match_picks_dl_google() {
        let decoded = decode_response(SAMPLE_XML).unwrap();
        let preferred = ["https://dl.google.com".to_string()];
        let mut download_url = decoded.urls[0].clone();
        for candidate in &decoded.urls {
            if preferred.iter().any(|h| candidate.starts_with(h)) {
                download_url = candidate.clone();
                break;
            }
        }
        assert_eq!(
            download_url,
            "https://dl.google.com/release2/chrome/83.0.4103.116_chrome_installer.exe"
        );
        let preferred_none = ["https://nowhere.invalid".to_string()];
        let mut download_url = decoded.urls[0].clone();
        for candidate in &decoded.urls {
            if preferred_none.iter().any(|h| candidate.starts_with(h)) {
                download_url = candidate.clone();
                break;
            }
        }
        assert_eq!(
            download_url,
            "https://dl.google.com/release2/chrome/83.0.4103.116_chrome_installer.exe"
        );
    }

    #[test]
    fn url_concat_no_slash_normalization() {
        let xml = r#"<response protocol="3.1"><app appid="{X}" status="ok"><updatecheck status="ok">
<manifest version="1.0"><packages><package name="inst.exe" hash_sha256="abc" size="1" required="true"/></packages></manifest>
<urls><url codebase="https://host/path"/></urls></updatecheck></app></response>"#;
        let decoded = decode_response(xml).unwrap();
        assert_eq!(decoded.urls, vec!["https://host/pathinst.exe"]);
    }

    #[test]
    fn size_non_digits_maps_to_none() {
        let xml = r#"<response><app><updatecheck>
<manifest version="1.0"><packages><package name="i.exe" size="1234x567"/></packages></manifest>
<urls><url codebase="https://h/"/></updatecheck></app></response>"#;
        let decoded = decode_response(xml).unwrap();
        assert_eq!(decoded.size, None);

        let xml = r#"<response><app><updatecheck>
<manifest version="1.0"><packages><package name="i.exe" size=""/></packages></manifest>
<urls><url codebase="https://h/"/></updatecheck></app></response>"#;
        let decoded = decode_response(xml).unwrap();
        assert_eq!(decoded.size, None);
    }

    #[test]
    fn exact_runtime_error_texts() {
        assert_eq!(
            decode_response("<response></response>")
                .unwrap_err()
                .to_string(),
            "No manifest found in Google Omaha response."
        );
        assert_eq!(
            decode_response("<response><manifest version=\"1\"/></response>")
                .unwrap_err()
                .to_string(),
            "No package found in Google Omaha response."
        );
        assert_eq!(
            decode_response(
                "<response><manifest version=\"1\"><package name=\"i.exe\"/></manifest></response>"
            )
            .unwrap_err()
            .to_string(),
            "No download URL found in Google Omaha response."
        );
    }

    #[test]
    fn unknown_channel_keyerror_text() {
        let config = json!({ "channel": "nope" });
        let err = get_package(&config).unwrap_err().to_string();
        assert_eq!(err, "Google Omaha channel 'nope' is not configured.");
    }

    #[test]
    fn request_overrides_channel_attributes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            use std::io::Read as _;
            let _ = stream.read(&mut buf);
            let body = "<html>not an omaha response</html>";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            use std::io::Write as _;
            stream.write_all(resp.as_bytes()).unwrap();
        });

        let config = json!({
            "channel": "win_beta_x64",
            "update_url": format!("http://127.0.0.1:{port}/fixture"),
            "request": { "os": "platform=\"win\" version=\"11.0\" arch=\"x64\"" }
        });
        let err = get_package(&config).unwrap_err().to_string();
        server.join().unwrap();
        assert_eq!(err, "No manifest found in Google Omaha response.");

        let config = json!({ "channel": "win_stable_x64", "request": { "os": "", "app": "" } });
        let err = get_package(&config).unwrap_err().to_string();
        assert!(err.contains("is not configured"), "unexpected: {err}");
    }

    #[test]
    fn request_body_is_frozen_template() {
        let body = build_request_body(
            "platform=\"win\" version=\"10.0\" sp=\"\" arch=\"x64\"",
            "appid=\"{8A69D345-D564-463C-AFF1-A69D9E530F96}\" version=\"\" nextversion=\"\"",
        );
        assert!(body.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<request protocol=\"3.0\" updater=\"Omaha\" updaterversion=\"1.3.36.372\" shell_version=\"1.3.36.352\" ismachine=\"0\""));
        assert!(body.contains("  <hw physmemory=\"16\" sse=\"1\" sse2=\"1\" sse3=\"1\" ssse3=\"1\" sse41=\"1\" sse42=\"1\" avx=\"1\"/>"));
        assert!(body.contains("  <os platform=\"win\" version=\"10.0\" sp=\"\" arch=\"x64\"/>"));
        assert!(body.contains("    <updatecheck/>"));
        assert!(body.contains("    <data name=\"install\" index=\"empty\"/>"));
        assert!(body.ends_with("</request>"));
        assert!(body.contains("nextversion=\"\""));
    }
}
