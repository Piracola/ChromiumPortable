//! Port of portable_builder/versions.py - version comparison and upgrade/release policy. Owner: Wave1-A.
//!
//! Semantics follow the Python source exactly, including its quirks:
//! - any non-numeric segment makes `compare_versions` report equality (Python
//!   catches the `ValueError` and returns 0 for the whole comparison),
//! - shorter versions are zero-padded, so `1.2 == 1.2.0`,
//! - `is_major_update`/`is_minor_update` compare segments as strings, not
//!   numbers (see the NOTE comments on each function).
//!
//! Porting notes live in docs/MIGRATION_RUST_TAURI.md S4.6.

use anyhow::{bail, Result};

/// Port of versions.py::compare_versions.
///
/// Returns 1 when `v1 > v2`, -1 when `v1 < v2`, 0 otherwise. Empty inputs and
/// versions containing any non-numeric segment compare as equal: Python's
/// falsy-input guard and its caught `ValueError` both return 0.
pub fn compare_versions(v1: &str, v2: &str) -> i32 {
    if v1.is_empty() || v2.is_empty() {
        return 0;
    }
    let (Some(parts1), Some(parts2)) = (parse_parts(v1), parse_parts(v2)) else {
        return 0;
    };
    for index in 0..parts1.len().max(parts2.len()) {
        let p1 = parts1.get(index).copied().unwrap_or(0);
        let p2 = parts2.get(index).copied().unwrap_or(0);
        if p1 > p2 {
            return 1;
        }
        if p1 < p2 {
            return -1;
        }
    }
    0
}

/// Python `int()` strips surrounding whitespace per segment and accepts signs,
/// which `i64::from_str` mirrors for the common cases. NOTE: Python ints are
/// arbitrary precision; here an overflowing segment is treated as non-numeric
/// (whole comparison returns 0), and unicode-digit segments are rejected.
fn parse_parts(version: &str) -> Option<Vec<i64>> {
    version
        .split('.')
        .map(|part| part.trim().parse::<i64>().ok())
        .collect()
}

/// Port of versions.py::major_version.
///
/// Returns the first dot-separated segment, or `None` for an empty version.
/// The segment is returned verbatim: it may be non-numeric ("beta") or empty
/// (".5" yields Some("")).
pub fn major_version(version: &str) -> Option<&str> {
    if version.is_empty() {
        return None;
    }
    version.split('.').next()
}

/// Port of versions.py::is_upgrade.
pub fn is_upgrade(new_version: &str, old_version: &str) -> bool {
    compare_versions(new_version, old_version) > 0
}

/// Port of versions.py::is_major_update.
///
/// NOTE quirk: majors are compared as strings, so "01" != "1" counts as a
/// major change. An empty first segment (".5") is falsy in Python and yields
/// false here too.
pub fn is_major_update(new_version: &str, old_version: &str) -> bool {
    let (Some(new_major), Some(old_major)) =
        (major_version(new_version), major_version(old_version))
    else {
        return false;
    };
    !new_major.is_empty() && !old_major.is_empty() && new_major != old_major
}

/// Port of versions.py::is_minor_update.
///
/// NOTE quirk: the comparison is on the raw version strings, so "1.2" vs
/// "1.2.0" counts as a minor update even though `compare_versions` calls
/// them equal.
pub fn is_minor_update(new_version: &str, old_version: &str) -> bool {
    let (Some(new_major), Some(old_major)) =
        (major_version(new_version), major_version(old_version))
    else {
        return false;
    };
    !new_major.is_empty()
        && !old_major.is_empty()
        && new_major == old_major
        && new_version != old_version
}

/// Port of versions.py::_segment_changed (private helper).
///
/// Python converts segments with bare `int()` and would raise ValueError on a
/// non-numeric segment; that path is unreachable from
/// `should_create_new_release` (guarded by `is_upgrade`), so the error is
/// kept for fidelity instead of being silently swallowed.
fn segment_changed(new_version: &str, old_version: &str, depth: usize) -> Result<bool> {
    let parts_new: Vec<&str> = new_version.split('.').collect();
    let parts_old: Vec<&str> = old_version.split('.').collect();
    for index in 0..depth {
        let p_new = parse_segment(&parts_new, index)?;
        let p_old = parse_segment(&parts_old, index)?;
        if p_new != p_old {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Missing segments pad with 0, mirroring Python's conditional expression.
fn parse_segment(parts: &[&str], index: usize) -> Result<i64> {
    match parts.get(index) {
        Some(part) => part
            .trim()
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("invalid literal for int() with base 10: {part:?}")),
        None => Ok(0),
    }
}

/// Port of versions.py::should_create_new_release.
///
/// Whether a version change should mint a new GitHub Release (the
/// `create_new_release_on` config key).
///
/// Policies:
/// - major (default): only when the first version segment changes. Fits
///   Chrome/Edge, where 147 -> 148 is a real major and same-major patches
///   should replace the latest assets in place.
/// - upgrade (alias `any`): any strictly newer version. Fits Helium, whose
///   first segment is permanently 0 and every 0.minor.patch is a distinct
///   upstream release.
/// - minor: strictly newer AND the first or second segment changed.
///
/// Unknown policies raise, like Python's ValueError. An empty version never
/// creates a release, before any policy validation (Python checks the falsy
/// guard first).
pub fn should_create_new_release(
    policy: Option<&str>,
    new_version: &str,
    old_version: &str,
) -> Result<bool> {
    if new_version.is_empty() || old_version.is_empty() {
        return Ok(false);
    }
    let policy = policy.unwrap_or("major");
    let normalized = policy.trim().to_lowercase();
    if normalized.is_empty() || normalized == "major" {
        return Ok(is_major_update(new_version, old_version));
    }
    if normalized == "upgrade" || normalized == "any" {
        return Ok(is_upgrade(new_version, old_version));
    }
    if normalized == "minor" {
        return Ok(
            is_upgrade(new_version, old_version) && segment_changed(new_version, old_version, 2)?
        );
    }
    bail!("Unknown create_new_release policy '{policy}'; expected major, upgrade, or minor");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_versions_compare_zero() {
        assert_eq!(compare_versions("1.2.3", "1.2.3"), 0);
        // Prefix relationships: shorter version is zero-padded.
        assert_eq!(compare_versions("147.0.1.0", "147.0.1"), 0);
        assert_eq!(compare_versions("1.2", "1.2.0"), 0);
        assert_eq!(compare_versions("1", "1.0.0.0"), 0);
    }

    #[test]
    fn shorter_version_padded_with_zeros() {
        assert_eq!(compare_versions("1.2", "1.2.1"), -1);
        assert_eq!(compare_versions("1.2.1", "1.2"), 1);
        // Numeric per-segment comparison, not lexicographic.
        assert_eq!(compare_versions("1.2", "1.10"), -1);
        assert_eq!(compare_versions("1.10", "1.9"), 1);
    }

    #[test]
    fn non_numeric_segment_compares_equal() {
        // Python catches the ValueError and returns 0 for the whole comparison.
        assert_eq!(compare_versions("1.2.beta", "1.2.3"), 0);
        assert_eq!(compare_versions("abc", "1.0"), 0);
        assert_eq!(compare_versions("1.2", "1.x.1"), 0);
        assert_eq!(compare_versions("v1.2", "1.2"), 0);
    }

    #[test]
    fn empty_versions_compare_equal() {
        assert_eq!(compare_versions("1.2", ""), 0);
        assert_eq!(compare_versions("", "1.2"), 0);
        assert_eq!(compare_versions("", ""), 0);
    }

    #[test]
    fn segments_may_carry_signs_and_whitespace_like_python_int() {
        // Python int() strips surrounding whitespace and accepts signs.
        assert_eq!(compare_versions("1. 5", "1.5"), 0);
        assert_eq!(compare_versions("1.-2", "1.0"), -1);
    }

    #[test]
    fn major_version_basics() {
        assert_eq!(major_version("1.2.3"), Some("1"));
        assert_eq!(major_version("beta.1"), Some("beta"));
        assert_eq!(major_version(""), None);
        // Empty first segment is falsy downstream in Python.
        assert_eq!(major_version(".5"), Some(""));
    }

    #[test]
    fn is_upgrade_matches_python() {
        assert!(is_upgrade("1.3", "1.2"));
        assert!(!is_upgrade("1.2", "1.3"));
        assert!(!is_upgrade("1.2", "1.2"));
        // Equal after zero padding, so not an upgrade.
        assert!(!is_upgrade("1.2", "1.2.0"));
        assert!(is_upgrade("0.5.2", "0.5.1"));
    }

    #[test]
    fn is_major_update_matches_python() {
        assert!(is_major_update("2.0", "1.9"));
        assert!(!is_major_update("1.10", "1.9")); // same major segment
        assert!(!is_major_update("1.2", "1.2.0"));
        assert!(!is_major_update(".5", "1.0")); // empty first segment is falsy
        assert!(!is_major_update("", "1.0"));
        // NOTE quirk: majors compared as strings, so "01" != "1".
        assert!(is_major_update("01.0", "1.0"));
    }

    #[test]
    fn is_minor_update_matches_python() {
        assert!(is_minor_update("1.2", "1.3"));
        assert!(is_minor_update("1.2", "1.1"));
        assert!(!is_minor_update("1.2", "2.0")); // different major
        assert!(!is_minor_update("1.2", "1.2"));
        // NOTE quirk: raw string inequality, so padding differences count.
        assert!(is_minor_update("1.2", "1.2.0"));
    }

    #[test]
    fn policy_default_is_major() {
        assert!(!should_create_new_release(None, "1.2.9", "1.2.8").unwrap());
        assert!(should_create_new_release(None, "148.0.1", "147.0.9").unwrap());
        // Same-major patches replace assets in place, no new release.
        assert!(!should_create_new_release(Some("major"), "147.1.0", "147.0.9").unwrap());
    }

    #[test]
    fn policy_normalizes_case_and_whitespace() {
        assert!(should_create_new_release(Some(""), "2.0", "1.9").unwrap());
        assert!(should_create_new_release(Some("  Major  "), "2.0", "1.9").unwrap());
        assert!(should_create_new_release(Some("MAJOR"), "1.9", "2.0").unwrap());
    }

    #[test]
    fn policy_upgrade_is_any_strictly_newer() {
        assert!(should_create_new_release(Some("upgrade"), "0.5.1", "0.5.0").unwrap());
        assert!(should_create_new_release(Some("any"), "0.5.1", "0.5.0").unwrap());
        assert!(!should_create_new_release(Some("upgrade"), "0.5.0", "0.5.1").unwrap());
        assert!(!should_create_new_release(Some("upgrade"), "0.5.0", "0.5.0").unwrap());
        assert!(!should_create_new_release(Some("upgrade"), "1.2", "1.2.0").unwrap());
    }

    #[test]
    fn policy_minor_requires_first_or_second_segment_change() {
        assert!(should_create_new_release(Some("minor"), "1.2.5", "1.1.9").unwrap());
        assert!(should_create_new_release(Some("minor"), "2.0", "1.9.9").unwrap());
        assert!(!should_create_new_release(Some("minor"), "1.2.5", "1.2.4").unwrap());
        assert!(!should_create_new_release(Some("minor"), "1.2", "2.0").unwrap());
    }

    #[test]
    fn empty_versions_never_create_releases_even_for_unknown_policy() {
        // Python checks the falsy guard before validating the policy, so an
        // unknown policy with empty versions returns False instead of raising.
        for policy in [
            None,
            Some("major"),
            Some("upgrade"),
            Some("minor"),
            Some("weekly"),
        ] {
            assert!(!should_create_new_release(policy, "", "1.0").unwrap());
            assert!(!should_create_new_release(policy, "1.0", "").unwrap());
        }
    }

    #[test]
    fn unknown_policy_raises_like_python_valueerror() {
        let err = should_create_new_release(Some("weekly"), "2.0", "1.0").unwrap_err();
        assert!(
            err.to_string()
                .contains("Unknown create_new_release policy 'weekly'"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn versions_reference_matrix_replay() {
        // Golden replay of the live-Python decision table captured in
        // _migration/pe-golden/versions_reference.json (round 36).
        let golden_path = std::path::Path::new("_migration/pe-golden/versions_reference.json");
        if !golden_path.exists() {
            eprintln!("golden not present in this checkout; skipping");
            return;
        }
        let golden: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(golden_path).expect("read golden"))
                .unwrap();

        for row in golden["compare"].as_array().unwrap() {
            let (a, b) = (row["a"].as_str().unwrap(), row["b"].as_str().unwrap());
            let expected_cmp = row["cmp"].as_i64().unwrap();
            let got = i64::from(compare_versions(a, b));
            assert_eq!(
                got, expected_cmp,
                "compare_versions({a}, {b}) = {got}, Python = {expected_cmp}"
            );
            let expected_upgrade = row["is_upgrade"].as_bool().unwrap();
            assert_eq!(
                is_upgrade(a, b),
                expected_upgrade,
                "is_upgrade({a}, {b}) diverges from Python"
            );
        }

        for row in golden["policy"].as_array().unwrap() {
            let policy = row["policy"].as_str();
            let up = row["up"].as_str().unwrap();
            let cur = row["cur"].as_str().unwrap();
            if let Some(expected) = row.get("result").and_then(|v| v.as_bool()) {
                let got = should_create_new_release(policy, up, cur)
                    .unwrap_or_else(|e| panic!("policy {policy:?} ({up} vs {cur}) errored: {e}"));
                assert_eq!(
                    got, expected,
                    "should_create_new_release({policy:?}, {up}, {cur}) diverges from Python"
                );
            } else if row.get("error").is_some() {
                assert!(
                    should_create_new_release(policy, up, cur).is_err(),
                    "policy {policy:?} ({up} vs {cur}) should error like Python"
                );
            }
        }
    }
}
