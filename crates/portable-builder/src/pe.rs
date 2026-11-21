use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::Result;
use pelite::image::{IMAGE_DIRECTORY_ENTRY_IMPORT, IMAGE_DIRECTORY_ENTRY_RESOURCE};

const MACHINE_X86: u16 = 0x014C;
const MACHINE_X64: u16 = 0x8664;
const MACHINE_ARM64: u16 = 0xAA64;

const FALLBACK_TRANSLATIONS: [(u16, u16); 3] =
    [(0x0409, 0x04B0), (0x0409, 0x04E4), (0x0804, 0x04B0)];

const VERSION_FIELDS: [&str; 6] = [
    "CompanyName",
    "FileDescription",
    "FileVersion",
    "OriginalFilename",
    "ProductName",
    "ProductVersion",
];

fn pe_layout(data: &[u8]) -> Result<()> {
    if data.len() < 0x40 || data.get(..2) != Some(b"MZ".as_slice()) {
        anyhow::bail!("Not a PE image (missing MZ signature).");
    }
    let pe_offset = match data.get(0x3C..0x40).map(<[u8; 4]>::try_from) {
        Some(Ok(bytes)) => u32::from_le_bytes(bytes) as usize,
        _ => anyhow::bail!("Not a PE image (missing PE signature)."),
    };
    if data.get(pe_offset..pe_offset + 4) != Some(b"PE\0\0".as_slice()) {
        anyhow::bail!("Not a PE image (missing PE signature).");
    }
    let coff = pe_offset + 4;
    let read_u16 = |offset: usize| -> Result<u16> {
        data.get(offset..offset + 2)
            .map(|s| u16::from_le_bytes([s[0], s[1]]))
            .ok_or_else(|| anyhow::anyhow!("Not a PE image (missing PE signature)."))
    };
    let read_u32 = |offset: usize| -> Result<u32> {
        data.get(offset..offset + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
            .ok_or_else(|| anyhow::anyhow!("Not a PE image (missing PE signature)."))
    };
    let section_count = read_u16(coff + 2)?;
    let optional_size = read_u16(coff + 16)?;
    let optional = coff + 20;
    let optional_magic = read_u16(optional)?;
    if optional_magic != 0x20B && optional_magic != 0x10B {
        anyhow::bail!("Unsupported PE optional header magic 0x{optional_magic:x}.");
    }
    let data_directories = if optional_magic == 0x20B {
        optional + 112
    } else {
        optional + 96
    };
    for index in 0..16 {
        let entry = data_directories + index * 8;
        read_u32(entry)?;
        read_u32(entry + 4)?;
    }
    let section_table = optional + optional_size as usize;
    for index in 0..section_count as usize {
        let entry = section_table + index * 40;
        for probe in [0, 8, 12, 16, 20] {
            read_u32(entry + probe)?;
        }
    }
    Ok(())
}

type PeFile<'a> = pelite::PeFile<'a>;

fn parse_pe(data: &[u8]) -> Result<PeFile<'_>> {
    pe_layout(data)?;
    pelite::PeFile::from_bytes(data)
        .map_err(|err| anyhow::anyhow!("Not a PE image (missing PE signature). {err}"))
}

pub fn read_pe_machine(path: &Path) -> Result<String> {
    let data = std::fs::read(path)?;
    let file = parse_pe(&data)?;
    let machine = file.file_header().Machine;
    Ok(match machine {
        MACHINE_X86 => "x86".to_string(),
        MACHINE_X64 => "x64".to_string(),
        MACHINE_ARM64 => "arm64".to_string(),
        other => format!("0x{other:04x}"),
    })
}

/// The DLL name strings from the PE import directory, in table order.
pub fn read_pe_import_names(path: &Path) -> Result<Vec<String>> {
    Ok(read_pe_import_names_with_section(path)?.0)
}

fn read_pe_import_names_with_section(path: &Path) -> Result<(Vec<String>, Option<String>)> {
    let data = std::fs::read(path)?;
    let file = parse_pe(&data)?;
    let dir = file
        .data_directory()
        .get(IMAGE_DIRECTORY_ENTRY_IMPORT)
        .ok_or_else(|| anyhow::anyhow!("Not a PE image (missing PE signature)."))?;
    if dir.VirtualAddress == 0 {
        return Ok((Vec::new(), None));
    }
    let import_section = section_for_rva(&file, dir.VirtualAddress).ok_or_else(|| {
        anyhow::anyhow!(
            "Import directory RVA 0x{:x} falls outside every section: {}",
            dir.VirtualAddress,
            path.display()
        )
    })?;
    let imports = file
        .imports()
        .map_err(|err| anyhow::anyhow!("Not a PE image (missing PE signature). {err}"))?;
    let mut names = Vec::new();
    for desc in imports {
        if let Ok(name) = desc.dll_name() {
            let text = match name.to_str() {
                Ok(text) => text.to_string(),
                Err(_) => String::from_utf8_lossy(name.c_str()).into_owned(),
            };
            names.push(text);
        }
    }
    Ok((names, Some(import_section)))
}

fn section_for_rva(file: &PeFile<'_>, rva: u32) -> Option<String> {
    for header in file.section_headers() {
        let span = header.VirtualSize.max(header.SizeOfRawData);
        if span != 0
            && rva >= header.VirtualAddress
            && rva < header.VirtualAddress.wrapping_add(span)
        {
            let trimmed = match header.Name.iter().rposition(|&b| b != 0) {
                Some(last) => &header.Name[..=last],
                None => &[],
            };
            let name: String = trimmed
                .iter()
                .map(|&b| {
                    if b.is_ascii() {
                        b as char
                    } else {
                        char::REPLACEMENT_CHARACTER
                    }
                })
                .collect();
            return Some(name);
        }
    }
    None
}

static ABSOLUTE_VERSION_DLL: std::sync::LazyLock<regex::bytes::Regex> =
    std::sync::LazyLock::new(|| {
        regex::bytes::Regex::new(r#"(?i)[A-Za-z]:\\[^\x00]*version\.dll"#)
            .expect("ABSOLUTE_VERSION_DLL regex is static and valid")
    });

/// The browser executable must load our injected, relatively-referenced
/// version.dll FIRST (setdll/Detours prepends it so the loader resolves it
/// before Chromium's system VERSION.dll), the first import must carry no
/// drive or leading slash, and the raw file must contain no absolute
/// `[A-Za-z]:\\...version.dll` string (absolute path = injection failed).
pub fn assert_portable_version_import(path: &Path) -> Result<Vec<String>> {
    assert_portable_version_import_named(path, "version.dll")
}

pub fn assert_portable_version_import_named(path: &Path, dll_name: &str) -> Result<Vec<String>> {
    let path_str = path.display();
    let (imports, import_section) = read_pe_import_names_with_section(path)?;
    let listed = if imports.is_empty() {
        "(none)".to_string()
    } else {
        imports.join(", ")
    };
    if imports.is_empty() {
        anyhow::bail!("{path_str} has no import directory; it cannot load {dll_name}.");
    }

    let first = &imports[0];
    let first_name = first
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("");
    if !first_name.eq_ignore_ascii_case(dll_name) {
        anyhow::bail!(
            "{path_str} does not load {dll_name} first, so DLL injection did not take effect. \
             Imports: {listed}"
        );
    }

    let bytes = first.as_bytes();
    let has_drive = (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
        || first.starts_with("\\\\");
    if has_drive || first.starts_with(['\\', '/']) {
        anyhow::bail!("{path_str} imports {dll_name} through a non-portable path: {first}");
    }

    let mut raw_matches: Vec<String> = ABSOLUTE_VERSION_DLL
        .find_iter(std::fs::read(path)?.as_slice())
        .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
        .collect();
    raw_matches.sort();
    raw_matches.dedup();
    if let Some(first_match) = raw_matches.first() {
        anyhow::bail!("{path_str} contains non-portable version.dll import: {first_match}");
    }

    crate::log_fmt::info(format!(
        "Import table verified (directory in '{}'): {listed}",
        import_section.as_deref().unwrap_or("")
    ));
    Ok(imports)
}

pub fn read_version_info(path: &Path) -> Result<BTreeMap<String, String>> {
    let data = std::fs::read(path)?;
    let file = parse_pe(&data)?;
    let mut result = BTreeMap::new();

    let Ok(resources) = file.resources() else {
        return Ok(result);
    };
    let Ok(version_info) = resources.version_info() else {
        return Ok(result);
    };

    let mut translations: Vec<(u16, u16)> = version_info
        .translation()
        .iter()
        .map(|lang| (lang.lang_id, lang.charset_id))
        .collect();
    translations.extend(FALLBACK_TRANSLATIONS);

    let mut seen = HashSet::new();
    for (language, codepage) in translations {
        if !seen.insert((language, codepage)) {
            continue;
        }
        let lang = pelite::resources::version_info::Language {
            lang_id: language,
            charset_id: codepage,
        };
        // Case-insensitive key resolution: Windows' VerQueryValueW matches the
        // table keys without case sensitivity (the opera installer writes
        // "productname" lowercase and Python still reads ProductName), but an
        // exact-name key always wins over a case-variant one. First hit wins
        // per field, exactly like pe.py's "if field in result: continue".
        let string_keys = version_info_keys(&version_info, lang);
        for field in VERSION_FIELDS {
            if result.contains_key(field) {
                continue;
            }
            let resolved = if string_keys.iter().any(|key| key == field) {
                field
            } else {
                string_keys
                    .iter()
                    .find(|key| key.eq_ignore_ascii_case(field))
                    .map(String::as_str)
                    .unwrap_or(field)
            };
            if let Some(value) = version_info.value(lang, resolved) {
                let value = value.trim();
                if !value.is_empty() {
                    result.insert(field.to_string(), value.to_string());
                }
            }
        }
    }
    Ok(result)
}

fn version_info_keys<'a>(
    version_info: &pelite::resources::version_info::VersionInfo<'a>,
    lang: pelite::resources::version_info::Language,
) -> Vec<String> {
    let mut keys = Vec::new();
    version_info.strings(lang, |key, _value| keys.push(key.to_string()));
    keys
}

pub struct PeResource {
    pub kind: String,
    pub name: String,
    pub language: String,
    pub data: Vec<u8>,
}

pub fn iter_pe_resources(path: &Path) -> Result<Vec<PeResource>> {
    let data = std::fs::read(path)?;
    let file = parse_pe(&data)?;
    let Some(dir) = file.data_directory().get(IMAGE_DIRECTORY_ENTRY_RESOURCE) else {
        return Ok(Vec::new());
    };
    if dir.VirtualAddress == 0 || dir.Size == 0 {
        return Ok(Vec::new());
    }
    let Ok(resources) = file.resources() else {
        return Ok(Vec::new());
    };
    let Ok(root) = resources.root() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut parts = Vec::new();
    walk_resource_dir(&root, &mut parts, &mut out);
    Ok(out)
}

fn walk_resource_dir(
    dir: &pelite::resources::Directory<'_>,
    parts: &mut Vec<String>,
    out: &mut Vec<PeResource>,
) {
    for entry in dir.entries() {
        let raw_name = entry.image().Name;
        let name = if raw_name & 0x8000_0000 == 0 {
            raw_name.to_string()
        } else {
            match entry.name() {
                Ok(name) => name.to_string(),
                Err(_) => continue,
            }
        };
        if entry.is_dir() {
            if let Ok(pelite::resources::Entry::Directory(child)) = entry.entry() {
                parts.push(name);
                walk_resource_dir(&child, parts, out);
                parts.pop();
            }
            continue;
        }
        if let Ok(pelite::resources::Entry::DataEntry(data_entry)) = entry.entry() {
            let size = data_entry.size();
            let bytes = data_entry.bytes().ok();
            if bytes.is_none_or(|b| b.len() != size) {
                continue;
            }
            let mut ids: [String; 3] = ["0".into(), "0".into(), "0".into()];
            for (i, part) in parts.iter().enumerate().take(3) {
                ids[i] = part.clone();
            }
            if parts.len() < 3 {
                ids[parts.len()] = name;
            }
            out.push(PeResource {
                kind: ids[0].clone(),
                name: ids[1].clone(),
                language: ids[2].clone(),
                data: bytes.unwrap_or_default().to_vec(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_pe(machine: u16, magic: u16) -> Vec<u8> {
        let mut data = vec![0u8; 0x200];
        data[..2].copy_from_slice(b"MZ");
        data[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        data[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        data[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
        let optional = 0x98usize;
        data[optional..optional + 2].copy_from_slice(&magic.to_le_bytes());
        data[optional + 60..optional + 64].copy_from_slice(&0x200u32.to_le_bytes());
        data[optional + 56..optional + 60].copy_from_slice(&0x200u32.to_le_bytes());
        data[optional + 108..optional + 110].copy_from_slice(&16u16.to_le_bytes());
        data
    }

    #[test]
    fn machine_mapping() {
        let dir = std::env::temp_dir().join(format!("pe_rs_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (machine, expected) in [
            (0x014C, "x86"),
            (0x8664, "x64"),
            (0xAA64, "arm64"),
            (0x1234, "0x1234"),
        ] {
            let p = dir.join(format!("m{machine:04x}.exe"));
            std::fs::write(&p, minimal_pe(machine, 0x20B)).unwrap();
            assert_eq!(read_pe_machine(&p).unwrap(), expected);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_pe_inputs_error_like_python() {
        let dir = std::env::temp_dir().join(format!("pe_rs_test2_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let p = dir.join("no_mz.bin");
        std::fs::write(&p, [0u8; 100]).unwrap();
        let err = read_pe_machine(&p).unwrap_err().to_string();
        assert_eq!(err, "Not a PE image (missing MZ signature).");

        let p = dir.join("truncated.bin");
        std::fs::write(&p, [0u8; 10]).unwrap();
        let err = read_pe_machine(&p).unwrap_err().to_string();
        assert_eq!(err, "Not a PE image (missing MZ signature).");

        let mut bad = vec![0u8; 0x100];
        bad[..2].copy_from_slice(b"MZ");
        let p = dir.join("no_pe_sig.bin");
        std::fs::write(&p, &bad).unwrap();
        let err = read_pe_machine(&p).unwrap_err().to_string();
        assert_eq!(err, "Not a PE image (missing PE signature).");

        let p = dir.join("bad_magic.bin");
        std::fs::write(&p, minimal_pe(0x8664, 0x107)).unwrap();
        let err = read_pe_machine(&p).unwrap_err().to_string();
        assert_eq!(err, "Unsupported PE optional header magic 0x107.");

        std::fs::remove_dir_all(&dir).ok();
    }

    const GOLDEN_SAMPLES: &[(&str, &str)] = &[
        ("csex", "bin/360csex_23.1.1253.64.exe"),
        ("installer", "bin/BraveBrowserStandaloneSetup.exe"),
        ("opera", "bin/Opera_135.0.5973.92_Setup_x64.exe"),
        ("thorium", "bin/thorium_AVX2_mini_installer.exe"),
        ("vivaldi", "bin/Vivaldi.8.2.4133.52.x64.exe"),
    ];

    fn golden_root() -> Option<std::path::PathBuf> {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let golden = repo.join("_migration/pe-golden/python_reference.json");
        let samples_exist = GOLDEN_SAMPLES
            .iter()
            .all(|(_, rel)| repo.join(rel).is_file());
        (golden.is_file() && samples_exist).then_some(repo)
    }

    fn assert_fixture(kind: &str) -> std::path::PathBuf {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let source = repo.join("setdll/setdll-x64.exe");
        if !source.is_file() {
            panic!("setdll/setdll-x64.exe missing; assert tests require it");
        }
        let mut data = std::fs::read(&source).unwrap();
        let first = read_pe_import_names(&source).unwrap();
        assert_eq!(first[0], "KERNEL32.dll");
        if kind != "wrong_first" {
            let needle = b"KERNEL32.dll\0SHE";
            let pos = data
                .windows(needle.len())
                .position(|w| w == needle)
                .expect("KERNEL32.dll descriptor name not found");
            data[pos..pos + 12].copy_from_slice(b"version.dll\0");
        }
        if kind == "absolute" {
            data.extend_from_slice(b"C:\\Windows\\system32\\version.dll");
        }
        let dir = std::env::temp_dir().join(format!("pe_assert_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join(format!("{kind}.exe"));
        std::fs::write(&out, &data).unwrap();
        out
    }

    #[test]
    fn assert_version_import_accepts_relative_first() {
        let path = assert_fixture("clean");
        let imports = assert_portable_version_import(&path).unwrap();
        assert_eq!(imports[0], "version.dll");
    }

    #[test]
    fn assert_version_import_rejects_wrong_first_import() {
        let path = assert_fixture("wrong_first");
        let err = assert_portable_version_import(&path)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("does not load version.dll first"),
            "unexpected error: {err}"
        );
        assert!(err.contains("Imports: KERNEL32.dll"), "unexpected: {err}");
    }

    #[test]
    fn assert_version_import_rejects_absolute_path_marker() {
        let path = assert_fixture("absolute");
        let err = assert_portable_version_import(&path)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("contains non-portable version.dll import: C:\\Windows"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn assert_version_import_no_imports_error_text() {
        let dir = std::env::temp_dir().join(format!("pe_assert_none_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("noimports.exe");
        std::fs::write(&path, minimal_pe(0x8664, 0x20B)).unwrap();
        let err = assert_portable_version_import(&path)
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            format!(
                "{} has no import directory; it cannot load version.dll.",
                path.display()
            )
        );
    }

    #[test]
    fn golden_machine_matches_python() {
        let Some(repo) = golden_root() else { return };
        let golden: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.join("_migration/pe-golden/python_reference.json"))
                .unwrap(),
        )
        .unwrap();
        for (key, rel) in GOLDEN_SAMPLES {
            let expected = golden[key]["machine"].as_str().unwrap();
            let actual = read_pe_machine(&repo.join(rel)).unwrap();
            assert_eq!(actual, expected, "machine mismatch for {rel}");
        }
    }

    #[test]
    fn shipped_component_machines_are_stable() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_eq!(
            read_pe_machine(&repo.join("setdll/setdll-x64.exe")).unwrap(),
            "x64"
        );
        assert_eq!(
            read_pe_machine(&repo.join("setdll/version-x64.dll")).unwrap(),
            "x64"
        );
        assert_eq!(
            read_pe_machine(&repo.join("setdll/setdll-arm64.exe")).unwrap(),
            "arm64"
        );
        assert_eq!(
            read_pe_machine(&repo.join("setdll/version-arm64.dll")).unwrap(),
            "arm64"
        );
    }

    #[test]
    fn golden_import_names_match_python() {
        let Some(repo) = golden_root() else { return };
        let golden: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.join("_migration/pe-golden/python_reference.json"))
                .unwrap(),
        )
        .unwrap();
        for (key, rel) in GOLDEN_SAMPLES {
            let expected: Vec<String> = golden[key]["import_names"][0]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            let actual = read_pe_import_names(&repo.join(rel)).unwrap();
            assert_eq!(actual, expected, "import names mismatch for {rel}");
        }
    }

    #[test]
    fn golden_version_info_matches_python() {
        let Some(repo) = golden_root() else { return };
        let golden: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.join("_migration/pe-golden/python_reference.json"))
                .unwrap(),
        )
        .unwrap();
        for (key, rel) in GOLDEN_SAMPLES {
            let expected = &golden[key]["version_info"];
            let actual = read_version_info(&repo.join(rel)).unwrap();
            let expected_map: BTreeMap<String, String> = expected
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .collect();
            assert_eq!(actual, expected_map, "version_info mismatch for {rel}");
        }
    }

    #[test]
    fn golden_resource_walk_matches_python() {
        let Some(repo) = golden_root() else { return };
        let golden: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.join("_migration/pe-golden/python_reference.json"))
                .unwrap(),
        )
        .unwrap();
        for (key, rel) in GOLDEN_SAMPLES {
            let expected = golden[key]["resources"].as_array().unwrap();
            let actual = iter_pe_resources(&repo.join(rel)).unwrap();
            assert_eq!(
                actual.len(),
                expected.len(),
                "resource count mismatch for {rel}"
            );
            for (i, (got, want)) in actual.iter().zip(expected.iter()).enumerate() {
                let want_kind = want[0].as_str().unwrap();
                let want_name = want[1].as_str().unwrap();
                let want_lang = want[2].as_str().unwrap();
                let want_size = want[3].as_u64().unwrap() as usize;
                assert_eq!(got.kind, want_kind, "resource[{i}] type mismatch for {rel}");
                assert_eq!(got.name, want_name, "resource[{i}] name mismatch for {rel}");
                assert_eq!(
                    got.language, want_lang,
                    "resource[{i}] lang mismatch for {rel}"
                );
                assert_eq!(
                    got.data.len(),
                    want_size,
                    "resource[{i}] size mismatch for {rel}"
                );
            }
        }
    }
}
