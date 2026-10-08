//! Port of portable_builder/i18n.py — UI language detection (zh-CN / en) and
//! wizard.json string loading. Owner: Wave1-B (migration doc S9; contract §6.5:
//! scripts/locales/wizard.json stays the single language source).
//!
//! Detection priority, identical to the Python module:
//! 1. WIZARD_LANG or CHROMIUMPORTABLE_LANG (first non-empty), normalized;
//! 2. Windows only: GetUserDefaultUILanguage, then GetUserDefaultLangID,
//!    both through LCID_MAP;
//! 3. LC_ALL / LC_MESSAGES / LANG, normalized;
//! 4. DEFAULT_LANG ("zh-CN").
//!
//! Python also consults locale.getlocale()[0] between steps 3 and 4. The C
//! runtime derives that value from the OS locale: on POSIX it duplicates what
//! LC_ALL/LC_MESSAGES/LANG already provide, and on Windows its value
//! ("English (United States)" style) never normalizes to a mapped id, so the
//! chain is observably identical without it. std has no portable equivalent;
//! deliberately not approximated.
//!
//! translate formatter (chosen approach): a tiny single-pass scanner over the
//! template bytes — no regex, no allocation on the failure path. It implements
//! exactly the str.format subset the wizard needs, with the same fallback
//! quirk: any formatting failure returns the template unchanged (Python wraps
//! str.format in except (KeyError, ValueError, IndexError)). Supported:
//! {name} (exact, whitespace-significant, repeatable) and {{ / }} escapes.
//! Everything else fails the format and keeps the template, matching Python
//! for: missing fields (KeyError), positional {} / {0} under kwargs-only calls
//! (IndexError), unclosed { and single } (ValueError). Documented divergences
//! for fields Python would either format or raise an uncaught error on —
//! conversions ({a!r}), format specs ({a:>5}, nested {a:>{w}}), attribute or
//! index access ({a.b}, {a[0]}) — all return the template unchanged here.
//! scripts/locales/wizard.json contains only simple {name} fields (verified),
//! so the wizard's observable behavior is unchanged.
//!
//! Other deliberate divergences: a non-object JSON document returns an empty
//! map (Python would raise AttributeError outside its catch tuple), and
//! non-string values inside a language table are skipped (Python would keep
//! them and str() them at format time).

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;

use serde_json::Value;

pub const DEFAULT_LANG: &str = "zh-CN";

/// Language tag -> UI language id (Python LANG_MAP, lowercase, with "_"
/// already folded to "-" before lookup).
pub const LANG_MAP: &[(&str, &str)] = &[
    ("zh", "zh-CN"),
    ("zh-cn", "zh-CN"),
    ("zh-hans", "zh-CN"),
    ("zh-sg", "zh-CN"),
    ("zh-tw", "zh-CN"),
    ("zh-hk", "zh-CN"),
    ("zh-hant", "zh-CN"),
    ("en", "en"),
];

/// Windows LCID -> UI language id (Python LCID_MAP).
pub const LCID_MAP: &[(u16, &str)] = &[
    (0x0804, "zh-CN"),
    (0x0404, "zh-CN"),
    (0x0C04, "zh-CN"),
    (0x1004, "zh-CN"),
    (0x1404, "zh-CN"),
    (0x0409, "en"),
    (0x0809, "en"),
];

pub fn lang_map_lookup(tag: &str) -> Option<&'static str> {
    LANG_MAP
        .iter()
        .find(|(key, _)| *key == tag)
        .map(|(_, id)| *id)
}

pub fn lang_from_lcid(lcid: u16) -> Option<&'static str> {
    LCID_MAP
        .iter()
        .find(|(key, _)| *key == lcid)
        .map(|(_, id)| *id)
}

/// Folds zh_CN / en-US / zh-Hant style input into a UI language id. None (or
/// empty) and unmapped tags yield None, like the Python Optional signature.
pub fn normalize_lang(raw: Option<&str>) -> Option<&'static str> {
    let raw = raw?;
    let text = raw.trim().replace('_', "-").to_lowercase();
    if let Some(id) = lang_map_lookup(&text) {
        return Some(id);
    }
    // Python: text.split("-", 1)[0]
    let primary = text.split('-').next().unwrap_or_default();
    lang_map_lookup(primary)
}

#[cfg(windows)]
fn detect_windows_ui_lang() -> Option<&'static str> {
    use windows_sys::Win32::Globalization::{GetUserDefaultLangID, GetUserDefaultUILanguage};

    // Same two-step probe as the Python ctypes calls: UI language first, then
    // the user default locale. Both return a 16-bit LANGID already, so the
    // Python masks are no-ops here. Neither call reports failure, which is
    // why the Python except-Exception net needs no analogue.
    let ui_lang = unsafe { GetUserDefaultUILanguage() };
    if let Some(id) = lang_from_lcid(ui_lang) {
        return Some(id);
    }
    let lang_id = unsafe { GetUserDefaultLangID() };
    lang_from_lcid(lang_id)
}

#[cfg(not(windows))]
fn detect_windows_ui_lang() -> Option<&'static str> {
    None
}

/// os.environ.get(name) with the Python truthiness the or-chain relies on:
/// unset or empty both count as absent. Non-UTF-8 values count as absent too.
fn env_non_empty(name: &str) -> Option<String> {
    match env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

pub fn detect_lang() -> &'static str {
    // Python: os.environ.get("WIZARD_LANG") or os.environ.get("CHROMIUMPORTABLE_LANG")
    let override_raw =
        env_non_empty("WIZARD_LANG").or_else(|| env_non_empty("CHROMIUMPORTABLE_LANG"));
    if let Some(id) = override_raw
        .as_deref()
        .and_then(|raw| normalize_lang(Some(raw)))
    {
        return id;
    }

    if let Some(id) = detect_windows_ui_lang() {
        return id;
    }

    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(id) = env_non_empty(name)
            .as_deref()
            .and_then(|raw| normalize_lang(Some(raw)))
        {
            return id;
        }
    }

    // Python's locale.getlocale() fallback is intentionally not ported; see
    // the module docs for why the observable chain is identical.
    DEFAULT_LANG
}

/// Reads a language pack from path_to_json_file (the FILE path, e.g.
/// scripts/locales/wizard.json), with the fallback table as the base and the
/// lang table merged on top. A directory path, missing file, bad UTF-8 or
/// malformed JSON all yield an empty map (Python swallows OSError/ValueError
/// around the read).
pub fn load_strings(
    path_to_json_file: &Path,
    lang: &str,
    fallback: &str,
) -> BTreeMap<String, String> {
    let mut strings = BTreeMap::new();
    let Ok(text) = fs::read_to_string(path_to_json_file) else {
        return strings;
    };
    let Ok(data) = serde_json::from_str::<Value>(&text) else {
        return strings;
    };

    let empty = serde_json::Map::new();
    // Value::get on a non-object document returns None, so a top-level array
    // or scalar degrades to the empty map instead of raising.
    let fallback_table = data
        .get(fallback)
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    for (key, value) in fallback_table {
        if let Some(text) = value.as_str() {
            strings.insert(key.clone(), text.to_string());
        }
    }
    if lang != fallback {
        let lang_table = data.get(lang).and_then(Value::as_object).unwrap_or(&empty);
        for (key, value) in lang_table {
            if let Some(text) = value.as_str() {
                strings.insert(key.clone(), text.to_string());
            }
        }
    }
    strings
}

/// load_strings with the Python default fallback=DEFAULT_LANG.
pub fn load_strings_default(path_to_json_file: &Path, lang: &str) -> BTreeMap<String, String> {
    load_strings(path_to_json_file, lang, DEFAULT_LANG)
}

/// translate(strings, key, **fields): a missing key (or an empty-string value)
/// falls back to the key itself; with no fields the template is returned
/// verbatim; with fields, any formatting failure returns the template
/// unchanged. Field values are pre-stringified by the caller.
pub fn translate(strings: &BTreeMap<String, String>, key: &str, fields: &[(&str, &str)]) -> String {
    let template = match strings.get(key) {
        Some(template) if !template.is_empty() => template.as_str(),
        _ => key,
    };
    if fields.is_empty() {
        return template.to_string();
    }
    format_template(template, fields).unwrap_or_else(|| template.to_string())
}

/// Single-pass str.format subset scanner. None means "format failed", which
/// translate maps to the unchanged template.
fn format_template(template: &str, fields: &[(&str, &str)]) -> Option<String> {
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
                out.push('{');
                i += 2;
            }
            b'}' if i + 1 < bytes.len() && bytes[i + 1] == b'}' => {
                out.push('}');
                i += 2;
            }
            b'{' => {
                // UTF-8 multibyte sequences never contain ASCII bytes, so
                // scanning for b'}' and slicing here stays on char bounds.
                let end = bytes[i + 1..]
                    .iter()
                    .position(|&b| b == b'}')
                    .map(|p| i + 1 + p)?;
                let name = simple_field_name(&template[i + 1..end])?;
                let value = fields
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, v)| *v)?;
                out.push_str(value);
                i = end + 1;
            }
            b'}' => return None,
            _ => {
                // i is always a char boundary: it only moves across ASCII
                // delimiter positions or whole chars.
                let ch = template[i..].chars().next()?;
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    Some(out)
}

/// Only bare {name} fields are substitutable; anything else (empty or
/// positional, conversion, format spec, and by extension whitespace variants
/// and attribute/index paths) fails the format so the template is kept —
/// Python raises KeyError/IndexError/ValueError on those under kwargs-only
/// calls.
fn simple_field_name(raw: &str) -> Option<&str> {
    if raw.is_empty() || raw.contains('!') || raw.contains(':') {
        return None;
    }
    if raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(raw)
}

/// make_translator(strings): closure binding the string table.
pub fn make_translator(
    strings: BTreeMap<String, String>,
) -> impl Fn(&str, &[(&str, &str)]) -> String {
    move |key, fields| translate(&strings, key, fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};

    fn env_lock() -> MutexGuard<'static, ()> {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        ENV_LOCK.lock().unwrap()
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
    }

    fn wizard_path() -> PathBuf {
        repo_root()
            .join("scripts")
            .join("locales")
            .join("wizard.json")
    }

    fn golden() -> Value {
        let path = repo_root()
            .join("_migration")
            .join("pe-golden")
            .join("config_i18n_reference.json");
        let text = fs::read_to_string(path).expect("golden reference file");
        serde_json::from_str(&text).expect("golden reference json")
    }

    fn strings_map<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn normalize_lang_matches_golden_samples() {
        let golden = golden();
        for (raw, expected) in golden["normalize_samples"]
            .as_object()
            .expect("normalize_samples")
        {
            // The "None" key in the golden stands for Python's None input.
            let input = if raw == "None" {
                None
            } else {
                Some(raw.as_str())
            };
            assert_eq!(normalize_lang(input), expected.as_str(), "raw={raw:?}");
        }
        assert_eq!(normalize_lang(None), None);
    }

    #[test]
    fn lang_tables_match_python_dicts() {
        assert_eq!(
            LANG_MAP,
            &[
                ("zh", "zh-CN"),
                ("zh-cn", "zh-CN"),
                ("zh-hans", "zh-CN"),
                ("zh-sg", "zh-CN"),
                ("zh-tw", "zh-CN"),
                ("zh-hk", "zh-CN"),
                ("zh-hant", "zh-CN"),
                ("en", "en"),
            ]
        );
        assert_eq!(lang_map_lookup("zh-hant"), Some("zh-CN"));
        assert_eq!(lang_map_lookup("en"), Some("en"));
        assert_eq!(lang_map_lookup("ja"), None);

        assert_eq!(
            LCID_MAP,
            &[
                (0x0804, "zh-CN"),
                (0x0404, "zh-CN"),
                (0x0C04, "zh-CN"),
                (0x1004, "zh-CN"),
                (0x1404, "zh-CN"),
                (0x0409, "en"),
                (0x0809, "en"),
            ]
        );
        assert_eq!(lang_from_lcid(0x0804), Some("zh-CN"));
        assert_eq!(lang_from_lcid(0x0409), Some("en"));
        assert_eq!(lang_from_lcid(0x0411), None); // ja-JP is not mapped in Python either
    }

    #[test]
    fn translate_three_quirks_match_golden() {
        let golden = golden();
        let quirk = &golden["translate_format_quirk"];
        let template = quirk["missing_field_returns_template"]
            .as_str()
            .expect("template");
        let strings = strings_map([("t", template)]);

        // Quirk 1: all fields present -> substituted.
        let ok = translate(&strings, "t", &[("count", "3"), ("arch", "x64")]);
        assert_eq!(ok, quirk["ok"].as_str().expect("ok"));

        // Quirk 2: a missing field returns the template unchanged.
        let missing = translate(&strings, "t", &[("count", "3")]);
        assert_eq!(
            missing,
            quirk["missing_field_returns_template"]
                .as_str()
                .expect("template")
        );

        // Quirk 3: a missing key returns the key itself.
        let nope = translate(&strings, "nope", &[]);
        assert_eq!(
            nope,
            quirk["missing_key_returns_key"]
                .as_str()
                .expect("missing key")
        );

        // Formatter details pinned to Python str.format semantics.
        assert_eq!(translate(&strings, "t", &[]), template); // no fields: verbatim, no scan
        assert_eq!(translate(&strings, "nope", &[("count", "3")]), "nope");

        let escapes = strings_map([("e", "{{a}} {a}")]);
        assert_eq!(translate(&escapes, "e", &[("a", "z")]), "{a} z");
        assert_eq!(translate(&escapes, "e", &[]), "{{a}} {a}"); // Python skips format without kwargs

        let repeated = strings_map([("r", "{a}-{a}")]);
        assert_eq!(translate(&repeated, "r", &[("a", "z")]), "z-z");

        let empty_value = strings_map([("k", "")]);
        assert_eq!(translate(&empty_value, "k", &[("a", "z")]), "k"); // "" is falsy in Python

        let whitespace = strings_map([("w", "{ a }")]);
        assert_eq!(translate(&whitespace, "w", &[("a", "z")]), "{ a }"); // KeyError ' a '

        let positional = strings_map([("p", "{0}")]);
        assert_eq!(translate(&positional, "p", &[("a", "z")]), "{0}"); // IndexError under kwargs

        let unclosed = strings_map([("u", "{a")]);
        assert_eq!(translate(&unclosed, "u", &[("a", "z")]), "{a"); // ValueError
        let lone_close = strings_map([("l", "}")]);
        assert_eq!(translate(&lone_close, "l", &[("a", "z")]), "}"); // ValueError
    }

    #[test]
    fn make_translator_binds_strings() {
        let t = make_translator(strings_map([("greeting", "hi {name}"), ("plain", "done")]));
        assert_eq!(t("greeting", &[("name", "b")]), "hi b");
        assert_eq!(t("plain", &[]), "done");
        assert_eq!(t("missing", &[("name", "b")]), "missing");
    }

    #[test]
    fn load_strings_real_wizard_file_matches_golden() {
        let golden = golden();
        let path = wizard_path();

        let en = load_strings(&path, "en", DEFAULT_LANG);
        let zh = load_strings_default(&path, "zh-CN");
        assert_eq!(
            en.len(),
            golden["strings_en_count"].as_u64().expect("en count") as usize
        );
        assert_eq!(
            zh.len(),
            golden["strings_zh_count"].as_u64().expect("zh count") as usize
        );

        let mut en_keys: Vec<&str> = en.keys().map(String::as_str).collect();
        en_keys.sort_unstable();
        let mut golden_keys: Vec<&str> = golden["strings_en_keys"]
            .as_array()
            .expect("en keys")
            .iter()
            .map(|value| value.as_str().expect("key string"))
            .collect();
        golden_keys.sort_unstable();
        assert_eq!(en_keys, golden_keys);

        for (key, expected) in golden["strings_en_sample"].as_object().expect("en sample") {
            let expected = expected.as_str().expect("sample string");
            assert_eq!(en[key].as_str(), expected, "key={key}");
        }
    }

    #[test]
    fn load_strings_missing_file_and_directory_return_empty() {
        let missing = repo_root()
            .join("scripts")
            .join("locales")
            .join("definitely_missing.json");
        assert!(load_strings(&missing, "en", DEFAULT_LANG).is_empty());

        let directory = repo_root().join("scripts").join("locales");
        assert!(load_strings(&directory, "en", DEFAULT_LANG).is_empty());
    }

    #[test]
    fn load_strings_fallback_then_lang_override() {
        let path = std::env::temp_dir().join(format!(
            "portable_builder_i18n_test_{}.json",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"{"zh-CN": {"a": "甲", "b": "乙"}, "en": {"b": "bee", "c": "cee"}}"#,
        )
        .expect("write temp locale file");

        let en = load_strings(&path, "en", DEFAULT_LANG);
        assert_eq!(en.get("a").map(String::as_str), Some("甲")); // from the fallback table
        assert_eq!(en.get("b").map(String::as_str), Some("bee")); // overridden by lang table
        assert_eq!(en.get("c").map(String::as_str), Some("cee"));
        assert_eq!(en.len(), 3);

        let zh = load_strings(&path, "zh-CN", DEFAULT_LANG);
        assert_eq!(zh.len(), 2); // lang == fallback: single table, no double merge

        let ja = load_strings(&path, "ja", DEFAULT_LANG); // table absent: pure fallback
        assert_eq!(ja.get("a").map(String::as_str), Some("甲"));
        assert_eq!(ja.len(), 2);

        fs::remove_file(&path).ok(); // leave no scratch files behind
    }

    #[test]
    fn detect_chain_env_overrides_match_golden() {
        let _guard = env_lock(); // env is process-global; keep tests serialized
        let golden = golden();

        const OVERRIDE_VARS: [&str; 5] = [
            "WIZARD_LANG",
            "CHROMIUMPORTABLE_LANG",
            "LC_ALL",
            "LC_MESSAGES",
            "LANG",
        ];
        struct Restore(Vec<(&'static str, Option<String>)>);
        impl Drop for Restore {
            fn drop(&mut self) {
                for (name, value) in &self.0 {
                    match value {
                        Some(saved) => env::set_var(name, saved),
                        None => env::remove_var(name),
                    }
                }
            }
        }
        let _restore = Restore(
            OVERRIDE_VARS
                .iter()
                .map(|name| (*name, env::var(name).ok()))
                .collect(),
        );
        for name in OVERRIDE_VARS {
            env::remove_var(name);
        }

        // Env override wins over everything (golden detect_en_override).
        env::set_var("WIZARD_LANG", "en");
        assert_eq!(
            detect_lang(),
            golden["detect_en_override"].as_str().expect("en")
        );

        // The override is normalized before use (golden detect_zhCN_normalize).
        env::set_var("WIZARD_LANG", "zh_CN");
        assert_eq!(
            detect_lang(),
            golden["detect_zhCN_normalize"].as_str().expect("zh-CN")
        );

        // First non-empty of the two overrides wins; "" is skipped.
        env::set_var("WIZARD_LANG", "");
        env::set_var("CHROMIUMPORTABLE_LANG", "ZH");
        assert_eq!(detect_lang(), "zh-CN");
        env::remove_var("CHROMIUMPORTABLE_LANG");

        // An unmapped override falls through the whole chain and lands exactly
        // where detection without any override lands. (Golden
        // detect_unmapped_falls_through is that value on the golden machine;
        // cross-machine the base itself varies with the OS locale, so the
        // portable assertion is fallthrough equality plus a valid id.)
        for name in OVERRIDE_VARS {
            env::remove_var(name);
        }
        let base = detect_lang();
        assert!(
            base == "zh-CN" || base == "en",
            "unexpected base id {base:?}"
        );
        env::set_var("WIZARD_LANG", "fr-FR");
        assert_eq!(detect_lang(), base, "unmapped override must fall through");

        // Same discipline for the LC_* chain: an unmapped value changes nothing.
        env::remove_var("WIZARD_LANG");
        env::set_var("LC_ALL", "not_a_real_locale");
        assert_eq!(detect_lang(), base);
        assert_posix_lc_chain();
    }

    #[cfg(not(windows))]
    fn assert_posix_lc_chain() {
        // On POSIX the Win32 step is skipped, so the LC_* chain is directly
        // observable once the overrides are cleared.
        env::set_var("LC_ALL", "zh_CN.UTF-8");
        assert_eq!(detect_lang(), "zh-CN");
        env::set_var("LC_ALL", "en_US.UTF-8");
        assert_eq!(detect_lang(), "en");
        env::remove_var("LC_ALL");
    }

    #[cfg(windows)]
    fn assert_posix_lc_chain() {}

    #[cfg(windows)]
    #[test]
    fn windows_ui_lang_probe_returns_mapped_id() {
        // Exercises the real Win32 bindings; None only means this machine's
        // LCIDs sit outside LCID_MAP, same as the Python ctypes probe.
        if let Some(id) = detect_windows_ui_lang() {
            assert!(id == "zh-CN" || id == "en");
        }
    }
}
