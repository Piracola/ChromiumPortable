//! Port of portable_builder/ini_overlay.py - three-layer chrome++.ini merge
//! (baseline setdll/chrome++.ini -> global chrome++.defaults.ini -> child repo
//! chrome++/chrome++.override.ini). Owner: Wave1-C.
//!
//! Contract: docs/MIGRATION_RUST_TAURI.md S4.4 - line-by-line merge preserving
//! comments and ordering, encoding probe order UTF-16 -> UTF-8-SIG -> UTF-8,
//! write back in the detected encoding, overriding a key the baseline does not
//! define is an ERROR (chrome++ silently ignores unknown keys, so a typo would
//! never surface). The "full ini shadows baseline" warning lives in the caller
//! (builder.py:resolve_chrome_plus_ini), not here.
//!
//! The ini has [general] / [tabs] / [keymapping] sections, but every key name is
//! unique across all of them, so a flat key map is unambiguous (module docstring
//! of ini_overlay.py).
//!
//! Python-fidelity notes (verified against CPython 3.12 by running the codec
//! probes, and against the CPython docs for the utf-16 codec):
//! - utf-16 decode: a FF FE BOM selects UTF-16LE, FE FF selects UTF-16BE, and
//!   with no BOM the data decodes as little-endian. CPython docs ("CPython
//!   implementation detail"): utf-16 decodes BOM-less data in native byte
//!   order; native order on every supported target (Windows x64) is LE. An odd
//!   byte count and unpaired surrogates raise UnicodeDecodeError and fall
//!   through to the next probe encoding.
//! - utf-16 encode: CPython's encoder writes a BOM in native byte order, i.e.
//!   always FF FE here, and does not special-case a leading U+FEFF in the text
//!   (a doubled BOM is possible, same as Python).
//! - newline="\r\n" translation: "\n" -> "\r\n", "\r" left untouched - a "\r\n"
//!   pair therefore re-encodes as "\r\r\n" (CPython io module).
//! - str.splitlines() splits on \r\n, \r, \n, \v, \f, \x1c, \x1d, \x1e, \x85,
//!   U+2028, U+2029; str.strip() trims Unicode whitespace plus \x1c..\x1f
//!   (which char::is_whitespace does not cover).

use std::collections::BTreeMap;
use std::path::Path;

/// Errors mirroring the Python exceptions of ini_overlay.py (OSError ->
/// [IniOverlayError::Io], ValueError -> Undecodable / NotAssignment / EmptyKey,
/// KeyError -> UnknownKeys).
#[derive(Debug, thiserror::Error)]
pub enum IniOverlayError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Unable to decode ini file: {0}")]
    Undecodable(String),
    #[error("Override line is not a key=value assignment: {0}")]
    NotAssignment(String),
    #[error("Override line has an empty key: {0:?}")]
    EmptyKey(String),
    #[error("chrome++.override.ini sets keys the baseline does not define: {unknown}. chrome++ silently ignores unknown keys, so this is treated as an error - check the spelling, or whether upstream removed the option.")]
    UnknownKeys { unknown: String },
}

/// The three probe encodings of ini_overlay.py:
/// ENCODINGS = ("utf-16", "utf-8-sig", "utf-8"). [IniEncoding::Utf16] mirrors
/// Python's utf-16 codec (BOM selects the endianness, no BOM decodes as
/// little-endian); [IniEncoding::Utf8Sig] strips/adds the UTF-8 BOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IniEncoding {
    /// Python utf-16 codec semantics; also the write_ini_text default
    /// (encoding="utf-16").
    #[default]
    Utf16,
    Utf8Sig,
    Utf8,
}

impl IniEncoding {
    pub fn name(self) -> &'static str {
        match self {
            Self::Utf16 => "utf-16",
            Self::Utf8Sig => "utf-8-sig",
            Self::Utf8 => "utf-8",
        }
    }

    /// Inverse of [IniEncoding::name] (Python passes encoding names around as
    /// strings).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "utf-16" => Some(Self::Utf16),
            "utf-8-sig" => Some(Self::Utf8Sig),
            "utf-8" => Some(Self::Utf8),
            _ => None,
        }
    }

    /// Strict Python decode; [Option::None] stands in for UnicodeDecodeError
    /// (the probe then falls through to the next encoding).
    fn decode(self, data: &[u8]) -> Option<String> {
        match self {
            Self::Utf16 => decode_python_utf16(data),
            Self::Utf8Sig => decode_strict_utf8(
                data.strip_prefix([0xEF, 0xBB, 0xBF].as_slice())
                    .unwrap_or(data),
            ),
            Self::Utf8 => decode_strict_utf8(data),
        }
    }

    /// Encode with Python open(..., encoding=..., newline="\r\n") semantics:
    /// "\n" -> "\r\n" while "\r" is left untouched (so a "\r\n" pair re-encodes
    /// as "\r\r\n" - verified against CPython), plus the codec BOM handling.
    pub fn encode(self, text: &str) -> Vec<u8> {
        let body = text.replace('\n', "\r\n");
        match self {
            Self::Utf16 => {
                let mut out = Vec::with_capacity((body.chars().count() + 1) * 2);
                // CPython utf-16 encoder writes the BOM in native byte order;
                // every supported target (Windows x64) is little-endian.
                out.extend_from_slice(&0xFEFF_u16.to_le_bytes());
                out.extend(body.encode_utf16().flat_map(u16::to_le_bytes));
                out
            }
            Self::Utf8Sig => {
                let mut out = Vec::with_capacity(body.len() + 3);
                out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
                out.extend_from_slice(body.as_bytes());
                out
            }
            Self::Utf8 => body.into_bytes(),
        }
    }
}

impl std::fmt::Display for IniEncoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Probe order of ini_overlay.py: ENCODINGS = ("utf-16", "utf-8-sig", "utf-8").
pub const ENCODING_PROBE_ORDER: [IniEncoding; 3] =
    [IniEncoding::Utf16, IniEncoding::Utf8Sig, IniEncoding::Utf8];

fn decode_strict_utf8(data: &[u8]) -> Option<String> {
    let (decoded, had_errors) = encoding_rs::UTF_8.decode_without_bom_handling(data);
    if had_errors {
        None
    } else {
        Some(decoded.into_owned())
    }
}

/// Python bytes.decode("utf-16") (CPython encodings/utf_16.py, verified against
/// Python 3.12): even byte count required; FF FE -> decode as UTF-16LE,
/// FE FF -> UTF-16BE, no BOM -> little-endian; the BOM is consumed (not part of
/// the text); unpaired surrogates raise UnicodeDecodeError.
fn decode_python_utf16(data: &[u8]) -> Option<String> {
    if !data.len().is_multiple_of(2) {
        return None;
    }
    let (little_endian, payload) = match data {
        [0xFF, 0xFE, rest @ ..] => (true, rest),
        [0xFE, 0xFF, rest @ ..] => (false, rest),
        _ => (true, data),
    };
    let units: Vec<u16> = payload
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    std::char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .ok()
}

/// str.isspace() set: Unicode White_Space plus U+001C..=U+001F (file/group/
/// record/unit separators), which char::is_whitespace does not cover.
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{001c}'..='\u{001f}')
}

/// str.strip() (both ends).
fn py_trim(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// Byte length of the leading str.lstrip() run (used to keep a baseline line
/// indentation when splicing an override in).
fn py_lstrip_len(s: &str) -> usize {
    s.find(|c: char| !is_py_space(c)).unwrap_or(s.len())
}

/// str.splitlines(): splits on \r\n (one boundary), \r, \n, \v (\x0b), \f (\x0c),
/// \x1c, \x1d, \x1e, \x85 (NEL), U+2028, U+2029. Unlike str::lines, a trailing
/// boundary does not produce a final "" element.
fn py_splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let boundary = match c {
            '\n' | '\u{000b}' | '\u{000c}' | '\u{001c}' | '\u{001d}' | '\u{001e}' | '\u{0085}'
            | '\u{2028}' | '\u{2029}' => c.len_utf8(),
            '\r' => {
                if matches!(chars.peek(), Some((_, '\n'))) {
                    chars.next();
                    2
                } else {
                    1
                }
            }
            _ => continue,
        };
        lines.push(&text[start..index]);
        start = index + boundary;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Port of ini_overlay.py:is_assignment - blank lines, comments (";"/"#", the
/// module COMMENT_PREFIXES) and [section] headers are not assignments;
/// everything else containing "=" is.
pub fn is_assignment(line: &str) -> bool {
    let stripped = py_trim(line);
    if stripped.is_empty() || stripped.starts_with([';', '#']) || stripped.starts_with('[') {
        return false;
    }
    stripped.contains('=')
}

/// Port of ini_overlay.py:parse_settings - (lowercase_key -> value) for every
/// assignment, ignoring comments. Flat map across sections (key names are
/// unique across all sections per the module docstring). Values are trimmed and
/// split on the FIRST "=" only (str.partition); inline comments are NOT
/// stripped - Python keeps them in the value. Python returns an
/// insertion-ordered dict but nothing observes that order, so a BTreeMap keeps
/// the Rust side deterministic.
pub fn parse_settings(text: &str) -> BTreeMap<String, String> {
    let mut settings = BTreeMap::new();
    for line in py_splitlines(text) {
        if !is_assignment(line) {
            continue;
        }
        let (key, value) = py_trim(line)
            .split_once('=')
            .expect("is_assignment guarantees a '='");
        settings.insert(py_trim(key).to_lowercase(), py_trim(value).to_string());
    }
    settings
}

/// One parsed override: the key exactly as declared in the override file plus
/// its trimmed value. Python keeps this as a (declared_key, value) tuple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Override {
    pub declared_key: String,
    pub value: String,
}

/// Port of ini_overlay.py:parse_overrides - (lowercase_key, Override) pairs
/// preserving the declared spelling. Re-declaring the same (case-insensitive)
/// key replaces the earlier entry in place, matching Python dict semantics
/// (first insertion position, last value). A non-assignment line without "=",
/// or with an empty key, is an error: a typo must not silently no-op.
pub fn parse_overrides(text: &str) -> Result<Vec<(String, Override)>, IniOverlayError> {
    let mut overrides: Vec<(String, Override)> = Vec::new();
    for raw in py_splitlines(text) {
        let line = py_trim(raw);
        if line.is_empty() || line.starts_with([';', '#']) || line.starts_with('[') {
            continue;
        }
        let Some((key_part, value)) = line.split_once('=') else {
            return Err(IniOverlayError::NotAssignment(raw.to_string()));
        };
        let key = py_trim(key_part);
        if key.is_empty() {
            return Err(IniOverlayError::EmptyKey(raw.to_string()));
        }
        let lower = key.to_lowercase();
        let parsed = Override {
            declared_key: key.to_string(),
            value: py_trim(value).to_string(),
        };
        match overrides
            .iter_mut()
            .find(|(existing, _)| *existing == lower)
        {
            Some((_, slot)) => *slot = parsed,
            None => overrides.push((lower, parsed)),
        }
    }
    Ok(overrides)
}

/// Port of ini_overlay.py:merge_ini - apply overrides onto base_text in place,
/// keeping comments and order. Each baseline assignment whose key matches a
/// remaining override is rewritten as
/// <original indent><declared_key>=<value>; application (and the returned
/// "applied" report of "declared_key=value" strings) follows baseline order.
/// Overriding a key the baseline does not define is an error listing the
/// unknown keys (KeyError in Python). The merged text joins lines with "\n"
/// plus one trailing "\n" exactly like Python's
/// "\n".join(lines) + "\n" (an empty baseline therefore yields "\n"); the write
/// path re-materializes CRLF.
pub fn merge_ini(
    base_text: &str,
    overrides: &[(String, Override)],
) -> Result<(String, Vec<String>), IniOverlayError> {
    let mut remaining = overrides.to_vec();
    let mut applied = Vec::new();
    let mut lines: Vec<String> = Vec::new();

    for raw in py_splitlines(base_text) {
        if is_assignment(raw) {
            let stripped = py_trim(raw);
            if let Some((key_part, _)) = stripped.split_once('=') {
                let key = py_trim(key_part).to_lowercase();
                if let Some(pos) = remaining.iter().position(|(lower, _)| *lower == key) {
                    let (_, applied_override) = remaining.remove(pos);
                    let indent = &raw[..py_lstrip_len(raw)];
                    lines.push(format!(
                        "{indent}{}={}",
                        applied_override.declared_key, applied_override.value
                    ));
                    applied.push(format!(
                        "{}={}",
                        applied_override.declared_key, applied_override.value
                    ));
                    continue;
                }
            }
        }
        lines.push(raw.to_string());
    }

    if !remaining.is_empty() {
        let unknown = remaining
            .iter()
            .map(|(_, o)| o.declared_key.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(IniOverlayError::UnknownKeys { unknown });
    }

    Ok((format!("{}\n", lines.join("\n")), applied))
}

/// Decode raw ini bytes with the Python probe order; [Option::None] stands for
/// Python ValueError("Unable to decode ini file: ...").
pub fn decode_ini_bytes(data: &[u8]) -> Option<(String, IniEncoding)> {
    for encoding in ENCODING_PROBE_ORDER {
        if let Some(text) = encoding.decode(data) {
            return Some((text, encoding));
        }
    }
    None
}

/// Port of ini_overlay.py:read_ini_text - (text, encoding). Upstream ships
/// UTF-16LE with a BOM, so real files decode as [IniEncoding::Utf16].
pub fn read_ini_text(path: impl AsRef<Path>) -> Result<(String, IniEncoding), IniOverlayError> {
    let path = path.as_ref();
    let data = std::fs::read(path)?;
    decode_ini_bytes(&data).ok_or_else(|| IniOverlayError::Undecodable(path.display().to_string()))
}

/// Port of ini_overlay.py:write_ini_text - writes back in the given (as
/// detected by [read_ini_text]) encoding with newline="\r\n" translation; see
/// [IniEncoding::encode]. Python defaults to encoding="utf-16"; [IniEncoding]
/// implements [Default] accordingly.
pub fn write_ini_text(
    path: impl AsRef<Path>,
    text: &str,
    encoding: IniEncoding,
) -> std::io::Result<()> {
    std::fs::write(path.as_ref(), encoding.encode(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("CARGO_MANIFEST_DIR sits at <repo>/crates/portable-builder")
            .to_path_buf()
    }

    fn read_fixture_bytes(rel: &str) -> Vec<u8> {
        fs::read(repo_root().join(rel)).expect("fixture exists")
    }

    fn temp_ini(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "portable-builder-ini-overlay-{}-{label}.tmp",
            std::process::id()
        ))
    }

    #[test]
    fn utf16_roundtrip() {
        // Write in the detected upstream encoding and read back faithfully.
        let text = "; comment\r\n[general]\r\ndata_dir=%app%\\..\\Data\r\nlone\rcarriage\n";
        let path = temp_ini("utf16-roundtrip");
        write_ini_text(&path, text, IniEncoding::Utf16).unwrap();
        let raw = fs::read(&path).unwrap();
        assert_eq!(
            &raw[..2],
            &[0xFF, 0xFE][..],
            "utf-16 always writes the LE BOM"
        );
        let (decoded, encoding) = read_ini_text(&path).unwrap();
        assert_eq!(encoding, IniEncoding::Utf16);
        // Write->read is NOT identity: the newline="\r\n" translation is baked into
        // the bytes ("\n" -> "\r\n", a "\r\n" pair -> "\r\r\n", lone "\r" kept).
        let translated: String = text.replace('\n', "\r\n");
        assert_eq!(decoded, translated);

        // Python 'utf-16' honors a UTF-16BE BOM on read (-> text "A").
        assert_eq!(
            IniEncoding::Utf16
                .decode(&[0xFE, 0xFF, 0x00, 0x41])
                .as_deref(),
            Some("A")
        );
        // No BOM -> little-endian (Python: b'a\x00'.decode('utf-16') == 'a').
        assert_eq!(
            IniEncoding::Utf16.decode(&[0x61, 0x00]).as_deref(),
            Some("a")
        );
        // Unpaired surrogate: UnicodeDecodeError -> probe falls through.
        assert_eq!(IniEncoding::Utf16.decode(&[0x00, 0xD8]), None);
        assert_eq!(
            decode_ini_bytes(&[0x00, 0xD8]),
            None,
            "invalid in every probe encoding"
        );

        // A leading U+FEFF in the text is NOT special-cased on write: the BOM is
        // always prepended, so a doubled BOM is possible (same as Python).
        assert_eq!(
            IniEncoding::Utf16.encode("\u{FEFF}x"),
            vec![0xFF, 0xFE, 0xFF, 0xFE, 0x78, 0x00]
        );

        // Odd-length bytes fail the utf-16 probe (UnicodeDecodeError) and are
        // picked up by utf-8-sig instead.
        let path = temp_ini("utf16-odd-fallback");
        fs::write(&path, b"abc").unwrap();
        let (decoded, encoding) = read_ini_text(&path).unwrap();
        assert_eq!(decoded, "abc");
        assert_eq!(encoding, IniEncoding::Utf8Sig);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn utf8_sig_roundtrip() {
        // Odd UTF-8 byte length so the utf-16 probe fails first (even-length ASCII
        // decodes as UTF-16 garbage without error in Python too: b'ab'.decode(
        // 'utf-16') == '\u{6261}').
        let text = "a\r\n";
        let path = temp_ini("utf8sig-roundtrip");
        write_ini_text(&path, text, IniEncoding::Utf8Sig).unwrap();
        let raw = fs::read(&path).unwrap();
        assert_eq!(
            &raw[..3],
            &[0xEF, 0xBB, 0xBF][..],
            "utf-8-sig writes the UTF-8 BOM"
        );
        let (decoded, encoding) = read_ini_text(&path).unwrap();
        assert_eq!(encoding, IniEncoding::Utf8Sig);
        // Write->read is not identity: "\n" became "\r\n" on disk.
        assert_eq!(decoded, "a\r\r\n");
        let _ = fs::remove_file(&path);

        // Plain utf-8 (no BOM) also detects as Utf8Sig: Python utf-8-sig decode
        // succeeds on any valid UTF-8, so the bare-utf-8 probe is only reached by
        // invalid input (which then fails too). Matches Python.
        let path = temp_ini("utf8-plain");
        // Odd byte length (a\r\r\n would be 4): "ab\r\n" encodes to 5 bytes,
        // so the utf-16 probe fails on parity and utf-8-sig wins.
        let plain = "ab\r\n";
        write_ini_text(&path, plain, IniEncoding::Utf8).unwrap();
        let raw = fs::read(&path).unwrap();
        assert_ne!(&raw[..3], &[0xEF, 0xBB, 0xBF][..]);
        let (decoded, encoding) = read_ini_text(&path).unwrap();
        assert_eq!(encoding, IniEncoding::Utf8Sig);
        assert_eq!(decoded, "ab\r\r\n");
        let _ = fs::remove_file(&path);

        // Even-length BOM-less ASCII really does mis-probe as UTF-16 (Python:
        // b'ab'.decode('utf-16') == '\u{6261}', no error).
        assert_eq!(
            IniEncoding::Utf16.decode(b"ab").as_deref(),
            Some("\u{6261}")
        );
    }

    #[test]
    fn merge_preserves_comment_lines_and_order() {
        // Hand-traced from the Python algorithm: comments, blank lines and
        // section headers pass through untouched; assignments are rewritten in
        // place; output ends with exactly one "\n" per line.
        let baseline =
            "; header comment\n[general]\ndata_dir=default\n# hash comment\n\n[tabs]\nwheel_tab=1\n";
        let overrides_text = "data_dir=%app%\\..\\Data\nwheel_tab=0\n";
        let (merged, applied) =
            merge_ini(baseline, &parse_overrides(overrides_text).unwrap()).unwrap();
        assert_eq!(
            merged,
            "; header comment\n[general]\ndata_dir=%app%\\..\\Data\n# hash comment\n\n[tabs]\nwheel_tab=0\n"
        );
        assert_eq!(applied, vec!["data_dir=%app%\\..\\Data", "wheel_tab=0"]);
    }

    #[test]
    fn merge_overwrites_existing_key() {
        // Baseline key in ANY case matches; the value runs to end of line (the
        // first "=" splits, so "=" inside values survives); the baseline line
        // indentation is kept; the output line uses the override declared
        // spelling. Traced to the Python output exactly.
        let baseline = "[general]\n  COMMAND_LINE = --flag=1 --x=2\n\nkeep_last_tab=1\n";
        let overrides_text = "command_line=--a=1 --b=2\nkeep_last_tab = 0\n";
        let (merged, applied) =
            merge_ini(baseline, &parse_overrides(overrides_text).unwrap()).unwrap();
        assert_eq!(
            merged,
            "[general]\n  command_line=--a=1 --b=2\n\nkeep_last_tab=0\n"
        );
        assert_eq!(applied, vec!["command_line=--a=1 --b=2", "keep_last_tab=0"]);

        // parse_settings partitions on the first "=" only (str.partition).
        let settings = parse_settings(baseline);
        assert_eq!(settings["command_line"], "--flag=1 --x=2");
        assert_eq!(settings["keep_last_tab"], "1");

        // Python does not strip inline comments: everything after the first "="
        // (trimmed) is the value.
        let settings = parse_settings("inline = v ; note\n");
        assert_eq!(settings["inline"], "v ; note");
    }

    #[test]
    fn merge_unknown_key_is_error() {
        let err = merge_ini(
            "a=1\n",
            &parse_overrides("a=ok\nmissing_one=\nno_such_key=v\n").unwrap(),
        )
        .expect_err("unknown override keys must fail the merge");
        // The empty-value override "missing_one=" parses fine but is unknown to
        // the baseline; remaining keys are listed in first-declaration order,
        // with the exact Python message.
        let message = err.to_string();
        let IniOverlayError::UnknownKeys { unknown } = err else {
            unreachable!()
        };
        assert_eq!(unknown, "missing_one, no_such_key");
        assert_eq!(
            message,
            "chrome++.override.ini sets keys the baseline does not define: missing_one, no_such_key. chrome++ silently ignores unknown keys, so this is treated as an error - check the spelling, or whether upstream removed the option."
        );
    }

    #[test]
    fn parse_overrides_errors() {
        // Python: ValueError("Override line is not a key=value assignment: ...")
        assert!(matches!(
            parse_overrides("good=1\nnot_an_assignment\n"),
            Err(IniOverlayError::NotAssignment(_))
        ));
        // Python: ValueError("Override line has an empty key: ...")
        assert!(matches!(
            parse_overrides("=value\n"),
            Err(IniOverlayError::EmptyKey(_))
        ));
        // Comments, blank lines and section headers are skipped.
        let overrides = parse_overrides("; c=1\n# d=2\n[sect]\n\ngood=1\n").unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].0, "good");
        assert_eq!(overrides[0].1.declared_key, "good");
        assert_eq!(overrides[0].1.value, "1");
    }

    #[test]
    fn applied_keys_reported_in_order() {
        // Application (and reporting) follows BASELINE order, not override
        // order; sections do not matter (flat key map).
        let baseline = "a=1\n[b]\nc=2\n\n[sections]\nb=3\n";
        let overrides_text = "b=x\nc=y\na=z\n";
        let (merged, applied) =
            merge_ini(baseline, &parse_overrides(overrides_text).unwrap()).unwrap();
        assert_eq!(merged, "a=z\n[b]\nc=y\n\n[sections]\nb=x\n");
        assert_eq!(applied, vec!["a=z", "c=y", "b=x"]);

        // Re-declared override key: last value wins, single application (Python
        // dict semantics: first position, last value).
        let (merged, applied) =
            merge_ini("k=1\n", &parse_overrides("k=first\nk=last\n").unwrap()).unwrap();
        assert_eq!(merged, "k=last\n");
        assert_eq!(applied, vec!["k=last"]);
    }

    #[test]
    fn crlf_preserved() {
        // Python write path (newline="\r\n"): "\n" -> "\r\n", "\r" untouched - a
        // "\r\n" pair re-encodes as "\r\r\n" (verified against CPython 3.12:
        // TextIOWrapper(newline="\r\n").write("a\r\nb\nc\rd")).
        let bytes = IniEncoding::Utf8.encode("a\r\nb\nc\rd");
        assert_eq!(&bytes[..], &b"a\r\r\nb\r\nc\rd"[..]);

        // merge output joins with "\n" (Python "\n".join); the write path
        // re-materializes CRLF, so merged fixture files stay CRLF on disk.
        let (merged, _) = merge_ini("a=1\r\nb=2\r\n", &parse_overrides("b=9\n").unwrap()).unwrap();
        assert_eq!(merged, "a=1\nb=9\n");
        let path = temp_ini("crlf");
        write_ini_text(&path, &merged, IniEncoding::Utf8Sig).unwrap();
        assert_eq!(
            &fs::read(&path).unwrap()[..],
            &[0xEF, 0xBB, 0xBF, 0x61, 0x3D, 0x31, 0x0D, 0x0A, 0x62, 0x3D, 0x39, 0x0D, 0x0A][..],
            "BOM + a=1\r\nb=9\r\n"
        );
    }

    #[test]
    fn real_fixtures_merge_and_reencode() {
        // Real repo fixtures: setdll/chrome++.ini (baseline, UTF-16LE BOM) and
        // setdll/chrome++.defaults.ini (global defaults layer, already in
        // override format). No chrome++.override.ini exists in-tree, so the
        // defaults file plays the override role here.
        let baseline_bytes = read_fixture_bytes("setdll/chrome++.ini");
        let defaults_bytes = read_fixture_bytes("setdll/chrome++.defaults.ini");

        let (baseline_text, baseline_encoding) =
            decode_ini_bytes(&baseline_bytes).expect("baseline decodes");
        assert_eq!(
            baseline_encoding,
            IniEncoding::Utf16,
            "upstream ships UTF-16LE with a BOM"
        );
        assert!(baseline_text.starts_with("; This file is the configuration file of Chrome++"));

        let (defaults_text, defaults_encoding) =
            decode_ini_bytes(&defaults_bytes).expect("defaults decodes");
        assert_eq!(defaults_encoding, IniEncoding::Utf16);
        let overrides = parse_overrides(&defaults_text).unwrap();
        let expected_defaults = [
            ("open_url_new_tab", "1"),
            ("open_bookmark_new_tab", "1"),
            ("wheel_tab_when_press_rbutton", "0"),
            ("suppress_false_upgrade_notification", "1"),
        ];
        assert_eq!(overrides.len(), expected_defaults.len());
        for ((lower, parsed), (key, value)) in overrides.iter().zip(expected_defaults) {
            assert_eq!(lower, key);
            assert_eq!(parsed.declared_key, key);
            assert_eq!(parsed.value, value);
        }

        let settings = parse_settings(&baseline_text);
        assert_eq!(
            settings.len(),
            22,
            "golden: 22 real assignments; the commented command_line examples are skipped"
        );
        assert_eq!(settings["data_dir"], "%app%\\..\\Data");
        assert_eq!(
            settings["command_line"], "",
            "the real assignment is empty; the =-bearing examples are comments"
        );
        for (lower, _) in &overrides {
            assert!(
                settings.contains_key(lower),
                "every override key exists in the baseline (unknown-key rule)"
            );
        }

        let baseline_lines: Vec<&str> = baseline_text.lines().collect();
        assert_eq!(baseline_lines.len(), 172);
        assert_eq!(baseline_lines[114], "suppress_false_upgrade_notification=0");
        assert_eq!(baseline_lines[148], "open_url_new_tab=0");

        // No-op merge: the merged text is the baseline with line endings
        // normalized to "\n" (Python "\n".join), and writing it back with the
        // detected encoding reproduces the ORIGINAL BYTES byte-exactly.
        let (noop, applied) = merge_ini(&baseline_text, &[]).unwrap();
        assert!(applied.is_empty());
        let expected_noop: String = baseline_text.chars().filter(|c| *c != '\r').collect();
        assert_eq!(noop, expected_noop);
        let path = temp_ini("golden-noop");
        write_ini_text(&path, &noop, baseline_encoding).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            baseline_bytes,
            "no-op merge roundtrips the real baseline byte-exactly"
        );
        let _ = fs::remove_file(&path);

        // Defaults merge onto the baseline: applied keys follow baseline order.
        let (merged, applied) = merge_ini(&baseline_text, &overrides).unwrap();
        assert_eq!(
            applied,
            vec![
                "suppress_false_upgrade_notification=1",
                "wheel_tab_when_press_rbutton=0",
                "open_url_new_tab=1",
                "open_bookmark_new_tab=1",
            ]
        );
        let lines: Vec<&str> = merged.lines().collect();
        assert_eq!(lines.len(), 172);
        assert_eq!(lines[25], "data_dir=%app%\\..\\Data");
        assert_eq!(lines[114], "suppress_false_upgrade_notification=1");
        assert_eq!(lines[147], "wheel_tab_when_press_rbutton=0");
        assert_eq!(lines[148], "open_url_new_tab=1");
        assert_eq!(lines[149], "open_bookmark_new_tab=1");

        // The merge emits "\n" only (no "\r"); the write path re-materializes
        // CRLF, and re-encoding with the detected encoding gives the expected
        // BOM prefixes: FF FE for UTF-16LE, EF BB BF for UTF-8-SIG.
        assert!(!merged.contains('\r'));
        assert!(merged.ends_with('\n'));
        assert_eq!(&IniEncoding::Utf16.encode(&merged)[..2], &[0xFF, 0xFE][..]);
        assert_eq!(
            &IniEncoding::Utf8Sig.encode(&merged)[..3],
            &[0xEF, 0xBB, 0xBF][..]
        );

        // Roundtrip the merged text through the detected encoding.
        let path = temp_ini("golden-merged");
        write_ini_text(&path, &merged, baseline_encoding).unwrap();
        let raw = fs::read(&path).unwrap();
        assert_eq!(&raw[..2], &[0xFF, 0xFE][..]);
        assert!(
            raw.windows(4).any(|w| w == &[0x0D, 0x00, 0x0A, 0x00][..]),
            "CRLF present in the UTF-16LE bytes"
        );
        let (roundtripped, _) = read_ini_text(&path).unwrap();
        // Write->read is not identity (newline translation is in the bytes).
        assert_eq!(roundtripped, merged.replace('\n', "\r\n"));
    }
}
