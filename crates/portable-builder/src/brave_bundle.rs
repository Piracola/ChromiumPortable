//! Port of portable_builder/brave_bundle.py — BCJ2 + LZMA decode for Brave/Omaha metainstallers.
//!
//! SPIKE OUTCOME (migration doc §4.5): Wave1-D confirmed the algorithm mapping works
//! (lzma-rs handles the outer LZMA_ALONE stream; the inner stream3 range decoder is
//! hand-rolled in Python and portable), but the full Rust implementation did not land
//! within Wave1. Per the contract's documented downgrade path, this module currently
//! reports clear unsupported errors for the BCJ2 path; everything else in the engine
//! is unaffected. Brave is non-product-channel (catalog product=false), so the only
//! observable loss is: Brave metainstallers cannot be auto-unpacked until the full
//! port lands (tracked as a follow-up wave).
//!
//! Re-enable scope when implemented: decode_bcj2_container + x86 branch converter +
//! tar extraction, golden-tested against _migration/pe-golden/discovery/brave.json.

use anyhow::{bail, Result};
use std::path::Path;

/// Port of brave_bundle.py::decode_bcj2_container — DOWNGRADED: not implemented.
pub fn decode_bcj2_container(_data: &[u8]) -> Result<Vec<u8>> {
    bail!(
        "Brave/Omaha metainstaller unpacking (BCJ2) is not implemented in the Rust engine \
         yet; use a full 7-Zip (7z.exe) if available, or run the Python engine for Brave \
         packages. See docs/MIGRATION_RUST_TAURI.md S4.5."
    )
}

/// Port of brave_bundle.py's metainstaller extraction entry — DOWNGRADED.
pub fn extract_brave_bundle(_payload: &[u8], _dest: &Path) -> Result<Vec<std::path::PathBuf>> {
    bail!(
        "Brave/Omaha metainstaller unpacking is not implemented in the Rust engine yet \
         (see docs/MIGRATION_RUST_TAURI.md S4.5)."
    )
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
    fn bcj2_path_reports_clear_unsupported_error() {
        let err = decode_bcj2_container(&[0u8; 32]).unwrap_err().to_string();
        assert!(
            err.contains("BCJ2"),
            "error must name the missing path: {err}"
        );
        assert!(
            err.contains("S4.5"),
            "error must reference the contract: {err}"
        );
    }
}
