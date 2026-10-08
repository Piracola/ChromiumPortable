//! Port of portable_builder/providers/ - package source providers (contract S4.3).
//! Owner: Wave2.

pub mod direct;
pub mod google_omaha;
pub mod microsoft_edge;
pub mod script;

use anyhow::Result;
use serde_json::Value;

/// Resolve a provider config into a package descriptor.
/// Mirrors providers/__init__.py::get_package - unknown type must list the
/// available provider names in the error.
pub fn get_package(_provider_config: &Value) -> Result<Value> {
    anyhow::bail!("providers::get_package is not implemented yet (Wave2, M2)")
}
