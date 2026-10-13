//! Port of portable_builder/providers/ - package source providers (contract S4.3).
//! Owner: Wave2.

pub mod direct;
pub mod google_omaha;
pub mod microsoft_edge;
pub mod script;

use anyhow::{bail, Result};
use serde_json::Value;

/// Resolve a provider config into a package descriptor.
/// Port of providers/__init__.py::get_package - dispatch on provider["type"],
/// unknown types must list the available provider names in the error
/// (KeyError text byte-mapped, carry-forward item 11).
pub fn get_package(provider_config: &Value) -> Result<Value> {
    let provider_type = provider_config
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("");
    match provider_type {
        "direct" => direct::get_package(provider_config),
        "google_omaha" => google_omaha::get_package(provider_config),
        "microsoft_edge" => microsoft_edge::get_package(provider_config),
        "script" => script::get_package(provider_config),
        other => bail!(
            "Unknown provider '{}'. Available providers: direct, google_omaha, microsoft_edge, script",
            other
        ),
    }
}
