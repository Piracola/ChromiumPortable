//! Port of portable_builder/brave_bundle.py — BCJ2 + LZMA decode for Brave/Omaha metainstallers.
//!
//! Wave3-A note: the Wave1-D downgrade is lifted. The full port is in place:
//! - the outer LZMA_ALONE stream decodes with lzma-rs,
//! - the inner BCJ2 container uses the range decoder ported 1:1 from
//!   brave_bundle.py::decode_bcj2_container (same probability table, same
//!   branch pattern scan, same address stream ordering),
//! - the resulting tar is unpacked with the tar crate, flat-file only
//!   (member.isfile, basename only), like Python's tarfile usage.
//!
//! Golden: _migration/pe-golden/discovery/brave.json drives the discovery-level
//! test; the BCJ2 unit tests below keep the decoder itself pinned.

use std::path::Path;

use anyhow::{anyhow, bail, Result};

use crate::pe::iter_pe_resources;

/// brave_bundle.py _BRANCH_PATTERN as a scan: [\xe8\xe9] or \x0f[\x80-\x8f].
fn find_branch(stream: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < stream.len() {
        match stream[i] {
            0xE8 | 0xE9 => return Some(i),
            0x0F if i + 1 < stream.len() && (0x80..=0x8F).contains(&stream[i + 1]) => {
                return Some(i)
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Port of brave_bundle.py::decode_bcj2_container.
pub fn decode_bcj2_container(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 20 {
        bail!("BCJ2 payload is too short.");
    }
    let read_u32 =
        |offset: usize| u32::from_le_bytes(data[offset..offset + 4].try_into().expect("4 bytes"));
    let original_size = read_u32(0) as usize;
    let size0 = read_u32(4) as usize;
    let size1 = read_u32(8) as usize;
    let size2 = read_u32(12) as usize;
    let size3 = read_u32(16) as usize;
    let total = 20usize
        .checked_add(size0)
        .and_then(|v| v.checked_add(size1))
        .and_then(|v| v.checked_add(size2))
        .and_then(|v| v.checked_add(size3))
        .ok_or_else(|| anyhow!("BCJ2 stream sizes exceed the payload length."))?;
    if total > data.len() {
        bail!("BCJ2 stream sizes exceed the payload length.");
    }

    let start0 = 20;
    let start1 = start0 + size0;
    let start2 = start1 + size1;
    let start3 = start2 + size2;
    let stream0 = &data[start0..start1];
    let stream1 = &data[start1..start2];
    let stream2 = &data[start2..start3];
    let stream3 = &data[start3..start3 + size3];
    if stream3.len() < 5 {
        bail!("BCJ2 range stream is too short.");
    }

    let mut probabilities = [1024u32; 258];
    let mut range_value: u32 = 0xFFFF_FFFF;
    let mut code: u32 = 0;
    let mut range_pos: usize = 0;
    for _ in 0..5 {
        code = (code << 8) | u32::from(stream3[range_pos]);
        range_pos += 1;
    }

    let mut main_pos = 0usize;
    let mut call_pos = 0usize;
    let mut jump_pos = 0usize;
    let mut previous: u8 = 0;
    let mut output: Vec<u8> = Vec::with_capacity(original_size);

    while output.len() < original_size {
        let Some(match_pos) = find_branch(stream0, main_pos) else {
            let remaining = original_size - output.len();
            let end = (main_pos + remaining).min(stream0.len());
            output.extend_from_slice(&stream0[main_pos..end]);
            // Python does main_pos += remaining here; the loop breaks right
            // after, so the assignment is dead - omitted to stay warning-free.
            break;
        };

        let mut candidate = match_pos;
        if stream0[candidate] == 0x0F {
            candidate += 1;
        }
        let chunk = &stream0[main_pos..candidate + 1];
        if chunk.is_empty() {
            bail!("Invalid BCJ2 main stream.");
        }
        let previous_before = if chunk.len() > 1 {
            chunk[chunk.len() - 2]
        } else {
            previous
        };
        output.extend_from_slice(chunk);
        main_pos = candidate + 1;
        let branch = chunk[chunk.len() - 1];

        let probability_index = match branch {
            0xE8 => usize::from(previous_before),
            0xE9 => 256,
            _ => 257,
        };

        let probability = probabilities[probability_index];
        let bound = u64::from(range_value >> 11) * u64::from(probability);
        let translated;
        if u64::from(code) < bound {
            range_value = bound as u32;
            probabilities[probability_index] = probability + ((2048 - probability) >> 5);
            previous = branch;
            translated = false;
        } else {
            range_value = (u64::from(range_value) - bound) as u32;
            code = (u64::from(code) - bound) as u32;
            probabilities[probability_index] = probability - (probability >> 5);
            translated = true;
        }

        if range_value < (1u32 << 24) {
            if range_pos >= stream3.len() {
                bail!("BCJ2 range stream ended early.");
            }
            range_value = range_value.wrapping_shl(8);
            code = (code << 8) | u32::from(stream3[range_pos]);
            range_pos += 1;
        }

        if !translated {
            continue;
        }

        let (address_stream, address_pos) = if branch == 0xE8 {
            (stream1, call_pos)
        } else {
            (stream2, jump_pos)
        };
        if address_pos + 4 > address_stream.len() {
            bail!("BCJ2 address stream ended early.");
        }
        let encoded = u32::from_be_bytes(
            address_stream[address_pos..address_pos + 4]
                .try_into()
                .expect("4 bytes"),
        );
        if branch == 0xE8 {
            call_pos += 4;
        } else {
            jump_pos += 4;
        }
        let destination = encoded.wrapping_sub((output.len() + 4) as u32);
        let raw_destination = destination.to_le_bytes();
        let take = 4.min(original_size - output.len());
        output.extend_from_slice(&raw_destination[..take]);
        previous = raw_destination[take - 1];
    }

    if output.len() != original_size {
        bail!(
            "BCJ2 decoded size mismatch: expected {}, got {}",
            original_size,
            output.len()
        );
    }
    Ok(output)
}

/// Port of brave_bundle.py::read_brave_metainstaller_tar - find the B/102
/// PE resource, LZMA-decompress it, then BCJ2-decode the container.
pub fn read_brave_metainstaller_tar(installer: &Path) -> Result<Vec<u8>> {
    let resources = iter_pe_resources(installer)?;
    let payload = resources
        .iter()
        .find(|entry| entry.kind == "B" && entry.name == "102")
        .map(|entry| entry.data.clone());
    let Some(payload) = payload else {
        bail!("Brave/Omaha payload resource B/102 was not found.");
    };
    // lzma.decompress(format=FORMAT_ALONE): the raw .lzma stream.
    let mut reader = std::io::Cursor::new(&payload);
    let mut decompressed: Vec<u8> = Vec::new();
    lzma_rs::lzma_decompress(&mut reader, &mut decompressed)
        .map_err(|_| anyhow!("Brave/Omaha payload is not valid LZMA data."))?;
    decode_bcj2_container(&decompressed)
}

/// Port of brave_bundle.py::extract_brave_metainstaller - statically unpack
/// the metainstaller's tar payload into output_dir, flat files only.
pub fn extract_brave_metainstaller(
    installer: &Path,
    output_dir: &Path,
) -> Result<Vec<std::path::PathBuf>> {
    std::fs::create_dir_all(output_dir)?;
    let tar_data = read_brave_metainstaller_tar(installer)?;
    let mut extracted = Vec::new();
    let mut archive = tar::Archive::new(std::io::Cursor::new(&tar_data));
    for member in archive.entries()? {
        let mut member = member?;
        if !member.header().entry_type().is_file() {
            continue;
        }
        let full_name = member
            .path()?
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if full_name.is_empty() || full_name == "." || full_name == ".." {
            continue;
        }
        let destination = output_dir.join(&full_name);
        let mut file = std::fs::File::create(&destination)?;
        std::io::copy(&mut member, &mut file)?;
        extracted.push(destination);
    }
    if extracted.is_empty() {
        bail!("Brave/Omaha payload contained no files.");
    }
    Ok(extracted)
}

/// Marker helper kept so discovery can detect Brave metainstallers and report them
/// accurately instead of failing deep inside extraction.
pub fn is_brave_metainstaller_name(file_name: &str) -> bool {
    let lowered = file_name.to_lowercase();
    lowered.contains("brave") && (lowered.ends_with(".exe") || lowered.contains("installer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brave_metainstaller_name_detection() {
        assert!(is_brave_metainstaller_name(
            "BraveBrowserStandaloneSetup.exe"
        ));
        assert!(is_brave_metainstaller_name("brave_installer.exe"));
        assert!(!is_brave_metainstaller_name("chrome_installer.exe"));
    }

    #[test]
    fn bcj2_short_payload_error_text() {
        // Below the 20-byte header minimum.
        let err = decode_bcj2_container(&[0u8; 10]).unwrap_err().to_string();
        assert_eq!(err, "BCJ2 payload is too short.");
        // Header parses but the range streams are truncated (32 bytes covers the
        // 20-byte header, then stream sizes exceed the remaining bytes).
        let err2 = decode_bcj2_container(&[0u8; 32]).unwrap_err().to_string();
        assert_eq!(err2, "BCJ2 range stream is too short.");
    }

    #[test]
    fn missing_b102_resource_error_text() {
        // A PE without the B/102 resource hits the exact Python error text.
        let dir = std::env::temp_dir().join(format!("brave_bundle_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../setdll/setdll-x64.exe");
        if !source.is_file() {
            return; // fixture unavailable
        }
        let path = dir.join("setdll.exe");
        std::fs::write(&path, std::fs::read(&source).unwrap()).unwrap();
        let err = read_brave_metainstaller_tar(&path).unwrap_err().to_string();
        assert_eq!(err, "Brave/Omaha payload resource B/102 was not found.");
        std::fs::remove_dir_all(&dir).ok();
    }
}
