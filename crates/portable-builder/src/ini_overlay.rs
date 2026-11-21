use std::collections::BTreeMap;
use std::path::Path;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IniEncoding {
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

    pub fn encode(self, text: &str) -> Vec<u8> {
        let body = text.replace('\n', "\r\n");
        match self {
            Self::Utf16 => {
                let mut out = Vec::with_capacity((body.chars().count() + 1) * 2);
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
        .as_chunks::<2>()
        .0
        .iter()
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

fn is_py_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{001c}'..='\u{001f}')
}

fn py_trim(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

fn py_lstrip_len(s: &str) -> usize {
    s.find(|c: char| !is_py_space(c)).unwrap_or(s.len())
}

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

pub fn is_assignment(line: &str) -> bool {
    let stripped = py_trim(line);
    if stripped.is_empty() || stripped.starts_with([';', '#']) || stripped.starts_with('[') {
        return false;
    }
    stripped.contains('=')
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Override {
    pub declared_key: String,
    pub value: String,
}

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

pub fn decode_ini_bytes(data: &[u8]) -> Option<(String, IniEncoding)> {
    for encoding in ENCODING_PROBE_ORDER {
        if let Some(text) = encoding.decode(data) {
            return Some((text, encoding));
        }
    }
    None
}

pub fn read_ini_text(path: impl AsRef<Path>) -> Result<(String, IniEncoding), IniOverlayError> {
    let path = path.as_ref();
    let data = std::fs::read(path)?;
    decode_ini_bytes(&data).ok_or_else(|| IniOverlayError::Undecodable(path.display().to_string()))
}

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
        let translated: String = text.replace('\n', "\r\n");
        assert_eq!(decoded, translated);

        assert_eq!(
            IniEncoding::Utf16
                .decode(&[0xFE, 0xFF, 0x00, 0x41])
                .as_deref(),
            Some("A")
        );
        assert_eq!(
            IniEncoding::Utf16.decode(&[0x61, 0x00]).as_deref(),
            Some("a")
        );
        assert_eq!(IniEncoding::Utf16.decode(&[0x00, 0xD8]), None);
        assert_eq!(
            decode_ini_bytes(&[0x00, 0xD8]),
            None,
            "invalid in every probe encoding"
        );

        assert_eq!(
            IniEncoding::Utf16.encode("\u{FEFF}x"),
            vec![0xFF, 0xFE, 0xFF, 0xFE, 0x78, 0x00]
        );

        let path = temp_ini("utf16-odd-fallback");
        fs::write(&path, b"abc").unwrap();
        let (decoded, encoding) = read_ini_text(&path).unwrap();
        assert_eq!(decoded, "abc");
        assert_eq!(encoding, IniEncoding::Utf8Sig);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn utf8_sig_roundtrip() {
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
        assert_eq!(decoded, "a\r\r\n");
        let _ = fs::remove_file(&path);

        let path = temp_ini("utf8-plain");
        let plain = "ab\r\n";
        write_ini_text(&path, plain, IniEncoding::Utf8).unwrap();
        let raw = fs::read(&path).unwrap();
        assert_ne!(&raw[..3], &[0xEF, 0xBB, 0xBF][..]);
        let (decoded, encoding) = read_ini_text(&path).unwrap();
        assert_eq!(encoding, IniEncoding::Utf8Sig);
        assert_eq!(decoded, "ab\r\r\n");
        let _ = fs::remove_file(&path);

        assert_eq!(
            IniEncoding::Utf16.decode(b"ab").as_deref(),
            Some("\u{6261}")
        );
    }

    #[test]
    fn merge_preserves_comment_lines_and_order() {
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
        let baseline = "[general]\n  COMMAND_LINE = --flag=1 --x=2\n\nkeep_last_tab=1\n";
        let overrides_text = "command_line=--a=1 --b=2\nkeep_last_tab = 0\n";
        let (merged, applied) =
            merge_ini(baseline, &parse_overrides(overrides_text).unwrap()).unwrap();
        assert_eq!(
            merged,
            "[general]\n  command_line=--a=1 --b=2\n\nkeep_last_tab=0\n"
        );
        assert_eq!(applied, vec!["command_line=--a=1 --b=2", "keep_last_tab=0"]);

        let settings = parse_settings(baseline);
        assert_eq!(settings["command_line"], "--flag=1 --x=2");
        assert_eq!(settings["keep_last_tab"], "1");

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
        assert!(matches!(
            parse_overrides("good=1\nnot_an_assignment\n"),
            Err(IniOverlayError::NotAssignment(_))
        ));
        assert!(matches!(
            parse_overrides("=value\n"),
            Err(IniOverlayError::EmptyKey(_))
        ));
        let overrides = parse_overrides("; c=1\n# d=2\n[sect]\n\ngood=1\n").unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].0, "good");
        assert_eq!(overrides[0].1.declared_key, "good");
        assert_eq!(overrides[0].1.value, "1");
    }

    #[test]
    fn applied_keys_reported_in_order() {
        let baseline = "a=1\n[b]\nc=2\n\n[sections]\nb=3\n";
        let overrides_text = "b=x\nc=y\na=z\n";
        let (merged, applied) =
            merge_ini(baseline, &parse_overrides(overrides_text).unwrap()).unwrap();
        assert_eq!(merged, "a=z\n[b]\nc=y\n\n[sections]\nb=x\n");
        assert_eq!(applied, vec!["a=z", "c=y", "b=x"]);

        let (merged, applied) =
            merge_ini("k=1\n", &parse_overrides("k=first\nk=last\n").unwrap()).unwrap();
        assert_eq!(merged, "k=last\n");
        assert_eq!(applied, vec!["k=last"]);
    }

    #[test]
    fn crlf_preserved() {
        let bytes = IniEncoding::Utf8.encode("a\r\nb\nc\rd");
        assert_eq!(&bytes[..], &b"a\r\r\nb\r\nc\rd"[..]);

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
        let assignments = py_splitlines(&baseline_text)
            .iter()
            .filter(|line| {
                let trimmed = py_trim(line);
                !trimmed.starts_with(';')
                    && trimmed.split_once('=').is_some_and(|(key, _)| {
                        let key = py_trim(key);
                        !key.is_empty()
                            && key.chars().all(|c| {
                                c.is_ascii_alphanumeric() || c == '_' || c == '[' || c == ']'
                            })
                    })
            })
            .count();
        assert_eq!(
            settings.len(),
            assignments,
            "commented command_line examples must not parse as assignments"
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
        assert_eq!(lines.len(), baseline_text.lines().count());
        for applied_line in &applied {
            let (key, _) = applied_line.split_once('=').unwrap();
            let hits = lines
                .iter()
                .filter(|line| {
                    let trimmed = line.trim_start();
                    !trimmed.starts_with(';')
                        && trimmed
                            .split_once('=')
                            .is_some_and(|(declared, _)| declared.trim() == key.trim())
                })
                .count();
            assert_eq!(hits, 1, "{key} must be declared exactly once after merging");
            assert!(
                lines.contains(&applied_line.as_str()),
                "missing {applied_line}"
            );
        }

        assert!(!merged.contains('\r'));
        assert!(merged.ends_with('\n'));
        assert_eq!(&IniEncoding::Utf16.encode(&merged)[..2], &[0xFF, 0xFE][..]);
        assert_eq!(
            &IniEncoding::Utf8Sig.encode(&merged)[..3],
            &[0xEF, 0xBB, 0xBF][..]
        );

        let path = temp_ini("golden-merged");
        write_ini_text(&path, &merged, baseline_encoding).unwrap();
        let raw = fs::read(&path).unwrap();
        assert_eq!(&raw[..2], &[0xFF, 0xFE][..]);
        assert!(
            raw.windows(4).any(|w| w == &[0x0D, 0x00, 0x0A, 0x00][..]),
            "CRLF present in the UTF-16LE bytes"
        );
        let (roundtripped, _) = read_ini_text(&path).unwrap();
        assert_eq!(roundtripped, merged.replace('\n', "\r\n"));
    }
}
