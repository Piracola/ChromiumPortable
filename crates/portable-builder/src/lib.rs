//! ChromiumPortable build engine - Rust port of the Python `portable_builder` package.
//!
//! Contract of record: docs/MIGRATION_RUST_TAURI.md
//! - S1: frozen CLI/env/log conventions (must not change)
//! - S4: per-module porting semantics
//! - S9: migration waves; each module names its owning wave in its doc comment.
//!
//! Crate rules:
//! - sync only, no async runtime (decision A, S2.2)
//! - no tauri/GUI dependencies

pub mod brave_bundle;
pub mod builder;
pub mod config;
pub mod discovery;
pub mod github_env;
pub mod ini_overlay;
pub mod log_fmt;
pub mod multi;
pub mod pe;
pub mod providers;
pub mod release;
pub mod tools;
pub mod verify;
pub mod versions;
