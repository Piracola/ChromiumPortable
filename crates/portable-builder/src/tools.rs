use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::log_fmt::{info, warn};

struct HashingWriter<'a, W: std::io::Write> {
    inner: W,
    digest: &'a mut Sha256,
    downloaded: &'a mut u64,
}
impl<W: std::io::Write> std::io::Write for HashingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.digest.update(&buf[..written]);
        *self.downloaded += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Mirror order is semantic (official source first).
pub const SEVEN_ZIP_URLS: [&str; 2] = [
    "https://www.7-zip.org/a/7zr.exe",
    "https://raw.githubusercontent.com/develar/7zip-bin/master/win/x64/7za.exe",
];

pub const LOCAL_7ZR: &str = "7zr.exe";

pub const SYSTEM_7Z_PATHS: [&str; 2] = [
    r"C:\Program Files\7-Zip\7z.exe",
    r"C:\Program Files (x86)\7-Zip\7z.exe",
];

pub const ABSOLUTE_VERSION_DLL_PATTERN: &str = r"[A-Za-z]:\\[^\x00]*version\.dll";

const CHUNK_SIZE: usize = 1024 * 1024;

/// Format bytes the way Windows Explorer does (1024-based, labelled B/KB/MB/GB).
/// `None`/`""` render as the empty string.
pub fn human_size(size: Option<f64>) -> String {
    let Some(mut value) = size else {
        return String::new();
    };
    for unit in ["B", "KB", "MB", "GB"] {
        if value < 1024.0 || unit == "GB" {
            return if unit == "B" {
                format!("{value:.0} {unit}")
            } else {
                format!("{value:.1} {unit}")
            };
        }
        value /= 1024.0;
    }
    format!("{value:.1} GB")
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut chunk = vec![0u8; CHUNK_SIZE];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

/// Accept hex, `sha256:<hex>` and base64 digests; return lowercase hex.
///
/// `None`/`""` map to `None`. The error text is byte-identical to Python's:
/// `Unrecognized SHA256 digest: {value!r}`.
pub fn normalize_sha256(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }

    let mut text = value.trim();
    if let Some(pos) = text.find(':') {
        text = text[pos + 1..].trim();
    }

    if let Some(hex_text) = try_hex_digest(text) {
        return Ok(Some(hex_text));
    }
    try_base64_digest(text)
        .map(Some)
        .map_err(|_| unrecognized_sha256_error(value))
}

fn try_hex_digest(text: &str) -> Option<String> {
    if text.len() != 64 {
        return None;
    }
    hex::decode(text).ok().map(hex::encode)
}

fn try_base64_digest(text: &str) -> Result<String> {
    let decoded = base64::engine::general_purpose::STANDARD.decode(text)?;
    if decoded.len() != 32 {
        bail!("digest is not 32 bytes");
    }
    Ok(hex::encode(decoded))
}

fn unrecognized_sha256_error(value: &str) -> anyhow::Error {
    anyhow!("Unrecognized SHA256 digest: {}", py_repr(value))
}

/// Python `repr()` of a str: single quotes by default, but double quotes when
/// the string contains ' and no " (repr's quote-selection rule); escapes \\,
/// \\n, \\r, \\t, the chosen quote, and non-printables as \\xNN.
fn py_repr(value: &str) -> String {
    let use_double = value.contains('\'') && !value.contains('"');
    let (quote, escaped_quote): (char, String) = if use_double {
        ('"', "\\\"".to_string())
    } else {
        ('\'', "\\'".to_string())
    };
    let mut out = String::with_capacity(value.len() + 2);
    out.push(quote);
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => out.push_str(&escaped_quote),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Size is checked against the on-disk file; digest against its content.
/// Prints `[INFO] SHA256 verified: ...` on success.
pub fn verify_file_digest(path: &Path, sha256: Option<&str>, size: Option<u64>) -> Result<()> {
    if let Some(size) = size {
        let actual_size = fs::metadata(path)?.len();
        if size != actual_size {
            bail!(
                "Size mismatch for {}: expected {} bytes, got {}",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default(),
                size,
                actual_size
            );
        }
    }

    if let Some(sha256) = sha256 {
        let expected = normalize_sha256(Some(sha256))?.ok_or_else(|| {
            anyhow!(
                "SHA256 mismatch for {}: expected None, got <not verified yet>",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
            )
        })?;
        let actual = sha256_file(path)?;
        if actual != expected {
            bail!(
                "SHA256 mismatch for {}: expected {}, got {}",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default(),
                expected,
                actual
            );
        }
        info(format!(
            "SHA256 verified: {} {}",
            path.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default(),
            actual
        ));
    }
    Ok(())
}

/// Streaming 1 MiB chunks, digest computed while downloading, size check,
/// and the cache-reject-resync semantics: an existing file is verified first
/// and deleted for re-download when the digest check rejects it.
///
/// `verify_ssl=false` maps to reqwest's `danger_accept_invalid_certs`.
/// NOTE: only the Edge provider may pass false (contract S3/S4.3).
pub fn download_file(
    url: &str,
    path: &Path,
    verify_ssl: bool,
    skip_existing: bool,
    sha256: Option<&str>,
    size: Option<u64>,
    warn_unverified: bool,
) -> Result<PathBuf> {
    if skip_existing && path.exists() {
        match verify_file_digest(path, sha256, size) {
            Ok(()) => {
                info(format!(
                    "File exists, skipping download: {}",
                    path.display()
                ));
                return Ok(path.to_path_buf());
            }
            Err(exc) => {
                warn(format!("Cached download rejected, fetching again: {exc}"));
                remove_path(path);
            }
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    info(format!("Downloading {url}"));

    // Python requests timeout=120 is PER-SOCKET-OP, not a total deadline. A total
    // timeout aborts slow multi-hundred-MB downloads mid-body (review-Wave2 finding 1).
    // Match the contract: no total limit; 120s connect timeout + TCP keepalive keep
    // dead-connection detection while allowing slow links to finish.
    let client = reqwest::blocking::Client::builder()
        .user_agent(crate::release::DEFAULT_USER_AGENT)
        .danger_accept_invalid_certs(!verify_ssl)
        .connect_timeout(std::time::Duration::from_secs(120))
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .build()?;
    let mut response = client.get(url).send()?.error_for_status()?;

    let mut digest = Sha256::new();
    let mut downloaded: u64 = 0;
    let file = fs::File::create(path)?;
    let mut hashing_writer = HashingWriter {
        inner: file,
        digest: &mut digest,
        downloaded: &mut downloaded,
    };
    std::io::copy(&mut response, &mut hashing_writer)?;
    drop(hashing_writer);
    drop(response);

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    if let Some(size) = size {
        if size != downloaded {
            remove_path(path);
            bail!("Size mismatch for {name}: expected {size} bytes, got {downloaded}");
        }
    }

    if let Some(sha256) = sha256 {
        let expected = normalize_sha256(Some(sha256))?.ok_or_else(|| {
            remove_path(path);
            anyhow!("SHA256 mismatch for {name}: expected None, got <digest of downloaded bytes>")
        })?;
        let actual = hex::encode(digest.finalize());
        if actual != expected {
            remove_path(path);
            bail!("SHA256 mismatch for {name}: expected {expected}, got {actual}");
        }
        info(format!("SHA256 verified: {name} {actual}"));
    } else if warn_unverified {
        warn(format!(
            "Upstream provided no SHA256 for {name}; downloaded bytes are unverified."
        ));
    }

    Ok(path.to_path_buf())
}

fn install_7z_with_chocolatey() -> Result<Option<String>> {
    if which("choco").is_none() {
        return Ok(None);
    }

    info("Trying to install 7-Zip with Chocolatey.");
    let output = Command::new("choco")
        .args(["install", "7zip", "-y", "--no-progress"])
        .output();
    let output = match output {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };
    if !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.status.success() {
        if !output.stderr.is_empty() {
            print!("{}", String::from_utf8_lossy(&output.stderr));
        }
        return Ok(None);
    }

    for path in SYSTEM_7Z_PATHS {
        if Path::new(path).exists() {
            return Ok(Some(path.to_string()));
        }
    }
    if which("7z").is_some() {
        return Ok(Some("7z".to_string()));
    }
    Ok(None)
}

/// shutil.which equivalent over PATH (+ PATHEXT semantics for .exe on Windows).
fn which(name: &str) -> Option<PathBuf> {
    let is_windows = cfg!(windows);
    let candidate_has_ext = name.contains('.') && !name.ends_with('.');
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let base = dir.join(name);
        if is_windows && !candidate_has_ext {
            for ext in [".com", ".exe", ".bat", ".cmd"] {
                let with_ext = PathBuf::from(format!("{}{}", base.display(), ext));
                if with_ext.is_file() {
                    return Some(with_ext);
                }
            }
        }
        if base.is_file() {
            return Some(base);
        }
    }
    None
}

/// Whether `find_7z_tool` may fetch 7-Zip itself. `Never` is the
/// inspect-package path: static inspection must not download or install.
pub enum SevenZipDownload {
    Never,
    Allowed,
}

/// Locate an extractor without changing the machine unless explicitly allowed.
///
/// The six-level discovery order is the contract (tools_7z_contract.md):
/// 1. system installs (both Program Files paths),
/// 2. workdir 7zr.exe,
/// 3. 7z on PATH,
/// 4. `allow_download=false` => hard error (inspect-package path: never
///    downloads or installs anything),
/// 5. download SEVEN_ZIP_URLS in order, deleting a failed attempt before
///    trying the next mirror,
/// 6. Chocolatey install when `allow_system_install` and choco is present.
pub fn find_7z_tool(
    workdir: &Path,
    allow_download: SevenZipDownload,
    allow_system_install: bool,
) -> Result<String> {
    for path in SYSTEM_7Z_PATHS {
        if Path::new(path).exists() {
            info(format!("Using system 7-Zip: {path}"));
            return Ok(path.to_string());
        }
    }

    let local_7zr = workdir.join(LOCAL_7ZR);
    if local_7zr.exists() {
        info(format!("Using local 7zr.exe: {}", local_7zr.display()));
        return Ok(local_7zr.to_string_lossy().into_owned());
    }

    if which("7z").is_some() {
        info("Using 7z from PATH");
        return Ok("7z".to_string());
    }

    if matches!(allow_download, SevenZipDownload::Never) {
        bail!(
            "7-Zip was not found. Static package inspection will not download or install it; \
             place 7zr.exe in the builder directory or install 7-Zip yourself."
        );
    }

    info("7-Zip not found. Downloading standalone extractor.");
    let mut last_error: Option<anyhow::Error> = None;
    for url in SEVEN_ZIP_URLS {
        match download_file(url, &local_7zr, true, false, None, None, false) {
            Ok(_) => return Ok(local_7zr.to_string_lossy().into_owned()),
            Err(exc) => {
                warn(format!("Failed to download 7-Zip from {url}: {exc}"));
                remove_path(&local_7zr);
                last_error = Some(exc);
            }
        }
    }

    if allow_system_install {
        let chocolatey_7z = install_7z_with_chocolatey()?;
        if let Some(chocolatey_7z) = chocolatey_7z {
            info(format!("Using Chocolatey-installed 7-Zip: {chocolatey_7z}"));
            return Ok(chocolatey_7z);
        }
    }

    bail!(
        "Unable to locate 7-Zip. Last download error: {}",
        last_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "None".to_string())
    );
}

/// Command is fixed: [7z, x, <archive>, -y, -o<outdir>]. Non-zero exit prints
/// the tool's stdout+stderr and raises `Extraction failed: <archive>`.
pub fn extract_with_7z(archive: &Path, output_dir: &Path, seven_zip_path: &str) -> Result<()> {
    fs::create_dir_all(output_dir)?;
    let cmd = [
        seven_zip_path,
        "x",
        &archive.to_string_lossy(),
        "-y",
        &format!("-o{}", output_dir.display()),
    ];
    info(format!("Extracting {}", archive.display()));
    let output = Command::new(cmd[0])
        .args(&cmd[1..])
        .output()
        .map_err(|exc| anyhow!("Extraction failed: {}: {exc}", archive.display()))?;
    if !output.status.success() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
        print!("{}", String::from_utf8_lossy(&output.stderr));
        bail!("Extraction failed: {}", archive.display());
    }
    Ok(())
}

/// Direct child first, then a recursive case-insensitive search. Returns None
/// when nothing matches.
pub fn find_child_dir(root: &Path, name: &str) -> Option<PathBuf> {
    let direct = root.join(name);
    if direct.exists() {
        return Some(direct);
    }

    let name_lower = name.to_lowercase();
    walk_entries(root).into_iter().find(|item| {
        item.is_dir()
            && item.file_name().map(|n| n.to_string_lossy().to_lowercase())
                == Some(name_lower.clone())
    })
}

/// Direct child first, then a recursive case-insensitive search. Returns None
/// when nothing matches.
pub fn find_child_file(root: &Path, name: &str) -> Option<PathBuf> {
    let direct = root.join(name);
    if direct.exists() {
        return Some(direct);
    }

    let name_lower = name.to_lowercase();
    walk_entries(root).into_iter().find(|item| {
        item.is_file()
            && item.file_name().map(|n| n.to_string_lossy().to_lowercase())
                == Some(name_lower.clone())
    })
}

fn walk_entries(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = read_dir.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        // rglob yields directory entries in os-scandir order; sort for a
        // deterministic result without changing the set of candidates.
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                stack.push(entry.clone());
            }
            out.push(entry);
        }
    }
    out
}

/// The preferred version dir when it exists, else the first direct child
/// whose name is dotted digits starting with a digit (e.g. "135.0.5973.92").
pub fn find_version_dir(root: &Path, preferred_version: Option<&str>) -> Option<PathBuf> {
    if let Some(preferred) = preferred_version {
        let candidate = root.join(preferred);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }

    let read_dir = fs::read_dir(root).ok()?;
    for item in read_dir.filter_map(|e| e.ok()).map(|e| e.path()) {
        if !item.is_dir() {
            continue;
        }
        let Some(name) = item.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        if name.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Some(item);
        }
    }
    None
}

/// Windows 上刚解出的 exe 常被杀软/索引器抓着句柄，remove_dir_all 会瞬时
/// os error 145/32；重试几轮再放弃。静默吞掉的错误调用方随后 rename/重建会
/// 把不可删除的残留暴露出来。
pub fn remove_path(path: &Path) {
    if path.is_dir() {
        for attempt in 0..4 {
            match fs::remove_dir_all(path) {
                Ok(()) => return,
                Err(_) if attempt < 3 => std::thread::sleep(Duration::from_millis(300)),
                Err(_) => return,
            }
        }
    } else if path.exists() {
        let _ = fs::remove_file(path);
    }
}

/// Reject exact file names that a portable target must never ship.
/// `forbidden` is the target's "forbidden_file_names" list (casefolded
/// comparison, posix relative paths in the error, sorted).
pub fn assert_no_forbidden_files(root: &Path, forbidden: &[String]) -> Result<()> {
    let forbidden: std::collections::HashSet<String> =
        forbidden.iter().map(|name| name.to_lowercase()).collect();
    if forbidden.is_empty() {
        return Ok(());
    }

    let mut matches: Vec<String> = walk_entries(root)
        .into_iter()
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .is_some_and(|name| forbidden.contains(&name))
        })
        .filter_map(|path| {
            path.strip_prefix(root)
                .ok()
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    matches.sort();
    if !matches.is_empty() {
        bail!("Build contains forbidden file(s): {}", matches.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_matches_python_output() {
        // Golden values captured from tools.py::human_size on this machine.
        assert_eq!(human_size(None), "");
        assert_eq!(human_size(Some(0.0)), "0 B");
        assert_eq!(human_size(Some(1.0)), "1 B");
        assert_eq!(human_size(Some(999.0)), "999 B");
        assert_eq!(human_size(Some(1023.0)), "1023 B");
        assert_eq!(human_size(Some(1024.0)), "1.0 KB");
        assert_eq!(human_size(Some(1536.0)), "1.5 KB");
        assert_eq!(human_size(Some(2.5 * 1024.0 * 1024.0)), "2.5 MB");
        assert_eq!(human_size(Some(148867748.0)), "142.0 MB");
        assert_eq!(human_size(Some(164784480.0)), "157.2 MB");
        assert_eq!(human_size(Some(151253128.0)), "144.2 MB");
        assert_eq!(human_size(Some(215226368.0)), "205.3 MB");
        assert_eq!(human_size(Some(1024.0 * 1024.0 * 1024.0)), "1.0 GB");
        assert_eq!(
            human_size(Some(5.0 * 1024.0 * 1024.0 * 1024.0 - 1.0)),
            "5.0 GB"
        );
        assert_eq!(human_size(Some(1024.0 * 1024.0)), "1.0 MB");
    }

    #[test]
    fn normalize_sha256_three_input_shapes() {
        let hex64 = "abcdef0123456789".repeat(4);
        assert_eq!(hex64.len(), 64);
        let upper: String = hex64.to_uppercase();
        assert_eq!(normalize_sha256(Some(&upper)).unwrap().unwrap(), hex64);
        assert_eq!(
            normalize_sha256(Some(&format!("sha256:{hex64}")))
                .unwrap()
                .unwrap(),
            hex64
        );
        assert_eq!(
            normalize_sha256(Some(&format!("sha256:{upper}")))
                .unwrap()
                .unwrap(),
            hex64
        );
        assert_eq!(normalize_sha256(Some(&hex64)).unwrap().unwrap(), hex64);
        let b64 = base64::engine::general_purpose::STANDARD.encode((0u8..32).collect::<Vec<_>>());
        assert_eq!(
            normalize_sha256(Some(&b64)).unwrap().unwrap(),
            hex::encode((0u8..32).collect::<Vec<_>>())
        );
        assert_eq!(normalize_sha256(None).unwrap(), None);
        assert_eq!(normalize_sha256(Some("")).unwrap(), None);
        let err = normalize_sha256(Some("   ")).unwrap_err().to_string();
        assert_eq!(err, "Unrecognized SHA256 digest: '   '");
    }

    #[test]
    fn normalize_sha256_error_text_matches_python() {
        let err = normalize_sha256(Some("not-a-digest"))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "Unrecognized SHA256 digest: 'not-a-digest'");
        let err = normalize_sha256(Some("abcdef")).unwrap_err().to_string();
        assert_eq!(err, "Unrecognized SHA256 digest: 'abcdef'");
        let b64 = base64::engine::general_purpose::STANDARD.encode([0u8; 31]);
        let err = normalize_sha256(Some(&b64)).unwrap_err().to_string();
        assert_eq!(
            err,
            "Unrecognized SHA256 digest: '<b64>'".replace("<b64>", &b64)
        );
        let err = normalize_sha256(Some("we'ird")).unwrap_err().to_string();
        assert_eq!(err, "Unrecognized SHA256 digest: \"we'ird\"");
        let err = normalize_sha256(Some("say \"hi\""))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "Unrecognized SHA256 digest: 'say \"hi\"'");
    }

    #[test]
    fn sha256_file_digests_streamed_content() {
        let dir = std::env::temp_dir().join("pb-tools-tests");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sha.bin");
        fs::write(&path, vec![0x61u8; 5 * 1024 * 1024 + 17]).unwrap();
        let digest = sha256_file(&path).unwrap();
        // Python golden: hashlib.sha256(b"a" * (5 * 1024 * 1024 + 17)).hexdigest()
        assert_eq!(
            digest,
            "f760459c08a89a1221abf8cc1a84d8a9af6af826fe173e94bbc51ffe9d83e2b9"
        );
        use sha2::Digest as _;
        let mut d = Sha256::new();
        d.update(vec![0x61u8; 5 * 1024 * 1024 + 17]);
        assert_eq!(digest, hex::encode(d.finalize()));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn download_and_cache_reject_resync_flow() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let payload = b"portable-builder fixture payload";
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = payload;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(resp.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
        });

        let dir = std::env::temp_dir().join("pb-tools-dl");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.bin");

        use sha2::Digest as _;
        let mut d = Sha256::new();
        d.update(payload);
        let good = hex::encode(d.finalize());
        let bad = "00".repeat(32);

        download_file(
            &format!("http://127.0.0.1:{port}/x"),
            &path,
            true,
            false,
            Some(&good),
            None,
            true,
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), payload);
        server.join().unwrap();

        let listener2 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port2 = listener2.local_addr().unwrap().port();
        let server2 = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener2.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = payload;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(resp.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        download_file(
            &format!("http://127.0.0.1:{port2}/x"),
            &path,
            true,
            true,
            Some(&good),
            None,
            true,
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), payload);

        fs::write(&path, b"tampered").unwrap();
        download_file(
            &format!("http://127.0.0.1:{port2}/x"),
            &path,
            true,
            true,
            Some(&good),
            None,
            true,
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), payload);

        let res = download_file(
            &format!("http://127.0.0.1:{port2}/x"),
            &path,
            true,
            false,
            Some(&bad),
            None,
            true,
        );
        assert!(res.is_err());
        assert!(!path.exists());
        server2.join().unwrap();

        let listener3 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port3 = listener3.local_addr().unwrap().port();
        let server3 = std::thread::spawn(move || {
            let (mut stream, _) = listener3.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = payload;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(resp.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
        });
        let res = download_file(
            &format!("http://127.0.0.1:{port3}/x"),
            &path,
            true,
            false,
            None,
            Some(12345),
            true,
        );
        let err = res.unwrap_err().to_string();
        assert_eq!(
            err,
            format!(
                "Size mismatch for fixture.bin: expected 12345 bytes, got {}",
                payload.len()
            )
        );
        assert!(!path.exists());
        server3.join().unwrap();

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_7z_tool_order_system_paths_first() {
        // The system paths cannot be created from a test (they need admin);
        // instead verify that a workdir 7zr.exe is found at level 2. Create a
        // fake 7zr.exe - find_7z_tool only stats it.
        //
        // Levels 1-3 depend on the host: a CI runner (or dev box) with a real
        // system 7-Zip legitimately resolves at level 1. So assert the
        // contract that holds everywhere: a tool IS found, and it is either
        // the host's system 7z or our workdir stub — never a download.
        let dir = std::env::temp_dir().join("pb-tools-7z");
        fs::create_dir_all(&dir).unwrap();
        let local = dir.join("7zr.exe");
        fs::write(&local, b"stub").unwrap();
        let found = find_7z_tool(&dir, SevenZipDownload::Never, false).unwrap();
        let found_path = Path::new(&found);
        let system_win = Path::new(crate::tools::SYSTEM_7Z_PATHS[0]);
        let host_system = found_path == system_win || {
            found_path
                .to_string_lossy()
                .eq_ignore_ascii_case(&system_win.to_string_lossy())
        };
        assert!(
            host_system || found_path == local.as_path(),
            "found {found} is neither the host system 7z nor the workdir stub"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_7z_tool_no_download_errors_like_python() {
        let empty = std::env::temp_dir().join("pb-tools-7z-none");
        fs::create_dir_all(&empty).unwrap();
        let local = empty.join("7zr.exe");
        fs::remove_file(&local).ok();

        // Only run the no-download assertion; levels 1-3 depend on the host
        // (a system 7-Zip or PATH 7z is legitimate). So gate on them missing.
        let host_has_tool =
            SYSTEM_7Z_PATHS.iter().any(|p| Path::new(p).exists()) || which("7z").is_some();
        if !host_has_tool {
            let err = find_7z_tool(&empty, SevenZipDownload::Never, false)
                .unwrap_err()
                .to_string();
            assert!(
                err.starts_with("7-Zip was not found. Static package inspection will not download or install it;"),
                "unexpected: {err}"
            );
        }
        fs::remove_dir_all(&empty).ok();
    }

    #[test]
    fn find_child_helpers_fall_back_to_case_insensitive_search() {
        let dir = std::env::temp_dir().join("pb-tools-child");
        let nested = dir.join("a").join("Nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("Marker.Exe"), b"x").unwrap();

        assert_eq!(
            find_child_dir(&dir, "nested").unwrap(),
            find_child_dir(&dir, "NESTED").unwrap()
        );
        assert_eq!(
            find_child_dir(&dir, "nested").unwrap().file_name().unwrap(),
            "Nested"
        );
        assert_eq!(
            find_child_file(&dir, "marker.exe").unwrap(),
            nested.join("Marker.Exe")
        );
        assert_eq!(find_child_dir(&dir, "missing"), None);
        assert_eq!(find_child_file(&dir, "missing"), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_version_dir_prefers_explicit_then_dotted_digits() {
        let dir = std::env::temp_dir().join("pb-tools-version");
        let v = dir.join("135.0.5973.92");
        let other = dir.join("App");
        fs::create_dir_all(&v).unwrap();
        fs::create_dir_all(&other).unwrap();

        assert_eq!(find_version_dir(&dir, None).unwrap(), v);
        assert_eq!(find_version_dir(&dir, Some("135.0.5973.92")).unwrap(), v);
        assert_eq!(find_version_dir(&dir, Some("999.0.0.0")).unwrap(), v);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remove_path_handles_file_dir_and_missing() {
        let dir = std::env::temp_dir().join("pb-tools-rm");
        let nested = dir.join("x").join("y");
        fs::create_dir_all(&nested).unwrap();
        let file = dir.join("f.txt");
        fs::write(&file, b"x").unwrap();

        remove_path(&file);
        assert!(!file.exists());
        remove_path(&dir);
        assert!(!dir.exists());
        // Missing path: no-op, no error.
        remove_path(&dir);
        remove_path(&file);
    }

    #[test]
    fn assert_no_forbidden_files_blocks_exact_casefolded_names() {
        let dir = std::env::temp_dir().join("pb-tools-forbid");
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("Evil.DLL"), b"x").unwrap();
        fs::write(sub.join("keep.txt"), b"x").unwrap();

        assert_no_forbidden_files(&dir, &[]).unwrap();
        assert_no_forbidden_files(&dir, &["other.dll".to_string()]).unwrap();
        let err = assert_no_forbidden_files(&dir, &["evil.dll".to_string()])
            .unwrap_err()
            .to_string();
        assert_eq!(err, "Build contains forbidden file(s): sub/Evil.DLL");

        let err = assert_no_forbidden_files(&dir, &["zzz".to_string(), "evil.dll".to_string()])
            .unwrap_err()
            .to_string();
        assert_eq!(err, "Build contains forbidden file(s): sub/Evil.DLL");
        fs::remove_dir_all(&dir).ok();
    }
}
