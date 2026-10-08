//! portable-builder CLI - frozen contract, docs/MIGRATION_RUST_TAURI.md S1/S5.
//! Subcommand names, flags, exit codes and log prefixes must stay byte-compatible
//! with the Python engine (`python -m portable_builder`). Implementations are
//! wired in M2/M3; until then every subcommand fails loudly.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "portable-builder",
    version,
    about = "Reusable portable Chromium browser builder"
)]
pub struct Cli {
    /// Path to browser config JSON/TOML
    #[arg(long, default_value = "browser.json")]
    pub config: String,
    /// Target name from config
    #[arg(long)]
    pub target: Option<String>,
    /// Caller repository working directory
    #[arg(long, default_value = ".")]
    pub workdir: String,
    /// Path to builder repository (auto-detected when omitted)
    #[arg(long)]
    pub builder_dir: Option<String>,
    #[command(subcommand)]
    pub command: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Check upstream and release versions
    Check,
    /// Build portable browser
    Build,
    /// Archive build/release into build/assets
    Archive,
    /// Extract the built archive and verify injection plus portability
    Verify {
        /// Archive path (default: newest match in build/assets)
        #[arg(long)]
        archive: Option<String>,
        /// Skip launching the browser; check the import table only
        #[arg(long)]
        no_smoke: bool,
    },
    /// Verify archives for multiple comma-separated targets
    VerifyTargets {
        /// Skip launching the browser; check the import table only
        #[arg(long)]
        no_smoke: bool,
    },
    /// Statically inspect an installer and locate its browser executable
    InspectPackage {
        /// Installer file or an already extracted directory
        package: String,
        /// Expected browser architecture
        #[arg(long, default_value = "x64")]
        architecture: String,
        /// Print the result as JSON
        #[arg(long)]
        json: bool,
        /// Keep the temporary extracted files for debugging
        #[arg(long)]
        keep_extracted: bool,
    },
    /// Locate, patch, and build a portable browser from an installer or extracted package
    BuildPackage {
        /// Installer file or extracted browser package directory
        package: String,
        #[arg(long, default_value = "x64")]
        architecture: String,
        /// Portable browser directory name
        #[arg(long)]
        output_dir: Option<String>,
        /// Also create a final 7z archive
        #[arg(long)]
        archive: bool,
    },
    /// Extract, locate, and statically inject Chrome++ into every installer in a directory
    BuildPackages {
        /// Directory containing installer packages
        directory: Option<String>,
        #[arg(long, default_value = "x64")]
        architecture: String,
        /// Also create final 7z archives and run static verify
        #[arg(long)]
        archive: bool,
    },
    /// Statically inspect every installer in a directory
    ResearchPackages {
        /// Directory containing installer samples
        directory: Option<String>,
        #[arg(long, default_value = "x64")]
        architecture: String,
        /// Print the combined result as JSON
        #[arg(long)]
        json: bool,
        /// Keep the temporary extracted files for debugging
        #[arg(long)]
        keep_extracted: bool,
    },
    /// Render release title/tag/body
    RenderRelease,
    /// Update existing GitHub release metadata and remove old assets
    UpdateRelease,
    /// Check multiple comma-separated targets
    CheckTargets {
        /// Comma-separated target names
        targets: String,
    },
    /// Build/archive updated comma-separated targets
    BuildTargets {
        /// Comma-separated target names
        targets: String,
    },
    /// Render release metadata for multiple targets
    RenderReleaseTargets {
        /// Comma-separated target names
        targets: String,
    },
    /// Update release for multiple targets
    UpdateReleaseTargets {
        /// Comma-separated target names
        targets: String,
    },
    /// Select one catalog target and write a single-target browser.json (was scripts/prepare_build_target.py)
    PrepareTarget {
        /// Target id from catalog/browser_catalog.json
        #[arg(long)]
        browser: String,
        /// Optional installer URL override (direct provider)
        #[arg(long)]
        url: Option<String>,
        /// Optional local installer path override
        #[arg(long)]
        path: Option<String>,
        #[arg(long, default_value = "build/selected.browser.json")]
        output: String,
        /// Override architecture
        #[arg(long)]
        architecture: Option<String>,
    },
    /// Resolve a browser's latest upstream installer (was scripts/upstream/resolve.py)
    ResolveUpstream {
        /// Pass-through arguments for upstream resolution
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(&cli) {
        portable_builder::log_fmt::fail(format!("{err:#}"));
        std::process::exit(1);
    }
}

fn run(cli: &Cli) -> Result<()> {
    // Wiring of real implementations happens wave by wave (M2/M3, S9).
    // The CLI surface itself is the frozen contract and is already complete.
    bail!(
        "subcommand '{}' is not implemented yet (migration in progress; \
         see docs/MIGRATION_RUST_TAURI.md S9 for the owning wave)",
        cli.command.name()
    );
}

impl Cmd {
    /// Canonical subcommand name, matching the Python CLI tokens one-for-one.
    fn name(&self) -> &'static str {
        match self {
            Cmd::Check => "check",
            Cmd::Build => "build",
            Cmd::Archive => "archive",
            Cmd::Verify { .. } => "verify",
            Cmd::VerifyTargets { .. } => "verify-targets",
            Cmd::InspectPackage { .. } => "inspect-package",
            Cmd::BuildPackage { .. } => "build-package",
            Cmd::BuildPackages { .. } => "build-packages",
            Cmd::ResearchPackages { .. } => "research-packages",
            Cmd::RenderRelease => "render-release",
            Cmd::UpdateRelease => "update-release",
            Cmd::CheckTargets { .. } => "check-targets",
            Cmd::BuildTargets { .. } => "build-targets",
            Cmd::RenderReleaseTargets { .. } => "render-release-targets",
            Cmd::UpdateReleaseTargets { .. } => "update-release-targets",
            Cmd::PrepareTarget { .. } => "prepare-target",
            Cmd::ResolveUpstream { .. } => "resolve-upstream",
        }
    }
}
