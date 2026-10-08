//! portable-builder CLI - frozen contract, docs/MIGRATION_RUST_TAURI.md S1/S5.
//! Subcommand names, flags, exit codes and log prefixes must stay byte-compatible
//! with the Python engine (`python -m portable_builder`). Implementations are
//! wired wave by wave (S9); until then every unwired subcommand fails loudly.
//! Wired so far: prepare-target (Wave2-C, port of scripts/prepare_build_target.py).

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use portable_builder::config::load_config;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

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
    // Wiring of real implementations happens wave by wave (S9).
    // The CLI surface itself is the frozen contract and is already complete.
    match &cli.command {
        Cmd::PrepareTarget {
            browser,
            url,
            path,
            output,
            architecture,
        } => prepare_target(
            cli.builder_dir.as_deref(),
            browser,
            url.as_deref(),
            path.as_deref(),
            architecture.as_deref(),
            Path::new(output),
        ),
        // Every remaining subcommand stays a loud fail until its wave lands.
        _ => bail!(
            "subcommand '{}' is not implemented yet (migration in progress; \
             see docs/MIGRATION_RUST_TAURI.md S9 for the owning wave)",
            cli.command.name()
        ),
    }
}

/// Resolve the builder repository directory (migration doc §6.5). Level 1 is the
/// explicit `--builder-dir` flag, level 2 the `PORTABLE_BUILDER_DIR` env var,
/// levels 3-5 probe the filesystem. Returns the first directory that actually
/// contains `catalog/browser_catalog.json`; if none does, falls back to the
/// last probed candidate so the error message names a concrete path.
fn resolve_catalog_path(builder_dir: Option<&str>) -> Result<PathBuf> {
    let candidates: Vec<PathBuf> = {
        let mut candidates = Vec::new();
        if let Some(dir) = builder_dir {
            candidates.push(PathBuf::from(dir));
        }
        if let Ok(dir) = std::env::var("PORTABLE_BUILDER_DIR") {
            candidates.push(PathBuf::from(dir));
        }
        // cwd probe (the "setdll probe" of resolve_builder_dir, adapted per §6.5)
        candidates.push(PathBuf::from("."));
        candidates.push(PathBuf::from("_portable_builder"));
        // exe-adjacent
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                candidates.push(parent.to_path_buf());
                candidates.push(parent.parent().map(Path::to_path_buf).unwrap_or_default());
            }
        }
        candidates
    };
    let mut fallback: Option<PathBuf> = None;
    for dir in candidates {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let catalog = dir.join("catalog").join("browser_catalog.json");
        if catalog.is_file() {
            return Ok(catalog);
        }
        fallback.get_or_insert(catalog);
    }
    let path = fallback.ok_or_else(|| {
        anyhow::anyhow!("no builder directory candidate could be derived for the catalog lookup")
    })?;
    Ok(path)
}

/// Port of scripts/prepare_build_target.py: select one catalog target and write
/// a single-target browser.json for CI/local use.
fn prepare_target(
    builder_dir: Option<&str>,
    browser: &str,
    url: Option<&str>,
    path: Option<&str>,
    architecture: Option<&str>,
    output: &Path,
) -> Result<()> {
    let catalog_path = resolve_catalog_path(builder_dir)?;
    write_selected(&catalog_path, browser, url, path, architecture, output)
}

/// Load the catalog and return the selected target (deep copy, unmutated).
/// Error text is byte-identical to the Python script:
/// `Unknown browser '<id>'. Known: <sorted list>`.
fn select_target_from_catalog(catalog_path: &Path, browser: &str) -> Result<Value> {
    let catalog = load_config(catalog_path).map_err(|err| anyhow::anyhow!("{err}"))?;
    let targets = catalog
        .get("targets")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let Some(source) = targets.get(browser) else {
        let mut known: Vec<&str> = targets.keys().map(String::as_str).collect();
        // Python sorted() over str keys == UTF-8 byte order == Rust str Ord.
        known.sort_unstable();
        bail!("Unknown browser '{browser}'. Known: {}", known.join(", "));
    };
    // Python: target = json.loads(json.dumps(target)) - a deep copy.
    Ok(source.clone())
}

/// Shared core: select + mutate the target, then write the single-target
/// browser.json. Used by both `run()` and the in-file tests.
fn write_selected(
    catalog_path: &Path,
    browser: &str,
    url: Option<&str>,
    path: Option<&str>,
    architecture: Option<&str>,
    output: &Path,
) -> Result<()> {
    let mut target = select_target_from_catalog(catalog_path, browser)
        .map_err(|err| anyhow::anyhow!("{err}"))?;

    if let Some(obj) = target.as_object_mut() {
        obj.insert("target".to_string(), json!(browser));
    }
    if let Some(arch) = architecture {
        if let Some(obj) = target.as_object_mut() {
            obj.insert("architecture".to_string(), json!(arch));
        }
    }
    if url.is_some() || path.is_some() {
        let mut provider = Map::new();
        provider.insert("type".to_string(), json!("direct"));
        provider.insert("verify_ssl".to_string(), json!(true));
        if let Some(u) = url {
            provider.insert("url".to_string(), json!(u));
        }
        if let Some(p) = path {
            provider.insert("path".to_string(), json!(p));
        }
        // Prefer explicit overrides over empty catalog placeholders.
        if let Some(obj) = target.as_object_mut() {
            obj.insert("provider".to_string(), Value::Object(provider));
        }
    } else if target
        .get("provider")
        .and_then(|p| p.get("type"))
        .and_then(Value::as_str)
        == Some("direct")
    {
        let provider = target.get("provider").expect("checked above");
        let has_url = provider
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
        let has_path = provider
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
        if !has_url && !has_path {
            bail!("Target '{browser}' needs --url or --path (no public auto-resolver).");
        }
    }

    let output = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()?.join(output)
    };
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut targets = Map::new();
    targets.insert(browser.to_string(), target.clone());
    let payload = json!({ "targets": targets });
    // Python: json.dumps(payload, ensure_ascii=False, indent=2) + "\n"
    std::fs::write(&output, serde_json::to_string_pretty(&payload)? + "\n")?;

    let product = if target
        .get("product")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "product"
    } else {
        "unofficial"
    };
    portable_builder::log_fmt::ok(format!("wrote {} ({browser}, {product})", output.display()));
    println!("{browser}");

    Ok(())
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
#[cfg(test)]
mod tests {
    use super::*;

    struct TempGuard {
        dir: PathBuf,
    }

    impl TempGuard {
        fn path(&self, name: &str) -> PathBuf {
            self.dir.join(name)
        }
    }

    impl Drop for TempGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn temp_dir(tag: &str) -> TempGuard {
        let dir = std::env::temp_dir().join(format!(
            "pe-prepare-target-tests-{}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        TempGuard { dir }
    }

    const MINI_CATALOG: &str = r#"{
  "targets": {
    "chrome_stable": {
      "name": "Chrome",
      "display_name": "Chrome++",
      "product": true,
      "output_dir": "Chrome",
      "exe_name": "chrome.exe",
      "architecture": "x64",
      "provider": {"type": "google_omaha", "channel": "win_stable_x64"},
      "version_root": "Chrome-bin",
      "layout": "move_version_root",
      "ini_location": "app_root",
      "version_dll_location": "app_root",
      "start_script": "\u5f00\u59cb.bat",
      "archive_name": "Chrome++_stable_{version}_{date}.7z",
      "release": {
        "tag": "v{version}",
        "title": "Chrome++ {version}",
        "asset_match": "stable",
        "body": "Chrome++ \u6784\u5efa\u7248\u672c\n\n\u6784\u5efa\u65f6\u95f4: {date}\nChrome \u7248\u672c: {version}\n"
      }
    },
    "brave_stable": {
      "name": "Brave",
      "product": false,
      "provider": {"type": "direct"},
      "architecture": "x64"
    },
    "zeta_stable": {"name": "Zeta"}
  }
}"#;

    fn mini_catalog_file(guard: &TempGuard) -> PathBuf {
        let dir = guard.path("repo").join("catalog");
        std::fs::create_dir_all(&dir).expect("mkdir catalog");
        let path = dir.join("browser_catalog.json");
        std::fs::write(&path, MINI_CATALOG).expect("write catalog");
        path
    }

    fn read_output(path: &Path) -> Value {
        let text = std::fs::read_to_string(path).expect("read output");
        serde_json::from_str(&text).expect("parse output json")
    }

    #[test]
    fn unknown_browser_error_text_is_exact() {
        let guard = temp_dir("unknown");
        let catalog = mini_catalog_file(&guard);
        let err = select_target_from_catalog(&catalog, "nope").expect_err("unknown browser");
        // Python: f"Unknown browser '{id}'. Known: {', '.join(sorted(targets))}"
        assert_eq!(
            err.to_string(),
            "Unknown browser 'nope'. Known: brave_stable, chrome_stable, zeta_stable"
        );
    }

    #[test]
    fn selected_output_shape_matches_golden() {
        // Golden: _migration/pe-golden/prepare_target_reference.json chrome_stable_full.
        let golden_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_migration/pe-golden/prepare_target_reference.json");
        let golden: Value =
            serde_json::from_str(&std::fs::read_to_string(&golden_path).expect("read golden"))
                .expect("parse golden");

        let guard = temp_dir("golden");
        let catalog = mini_catalog_file(&guard);
        let target = select_target_from_catalog(&catalog, "chrome_stable").expect("selected");

        // Deep-equal against the golden full record (all fields present, byte
        // semantics for the Chinese release body preserved via serde).
        assert_eq!(target, golden["chrome_stable_full"]);
        // product boolean semantics: true must stay a JSON boolean, not a string.
        assert_eq!(target["product"], json!(true));

        // The written payload shape: {"targets": {id: target}} only.
        let output = guard.path("build").join("selected.browser.json");
        write_selected(&catalog, "chrome_stable", None, None, None, &output).expect("write");
        let payload = read_output(&output);
        let obj = payload.as_object().expect("payload object");
        assert_eq!(obj.len(), 1, "payload must contain exactly the targets key");
        let target_ids: Vec<&String> = payload["targets"]
            .as_object()
            .expect("targets object")
            .keys()
            .collect();
        assert_eq!(target_ids, ["chrome_stable"]);
        // The written record carries the injected "target" key (Python:
        // target["target"] = args.browser); the golden full record is the raw
        // catalog dict without it. Compare field-for-field after removing it.
        let written = payload["targets"]["chrome_stable"].clone();
        let mut raw = written.as_object().expect("target object").clone();
        let injected = raw.remove("target").expect("injected target key");
        assert_eq!(injected, json!("chrome_stable"));
        assert_eq!(Value::Object(raw), golden["chrome_stable_full"]);
        // File ends with exactly one trailing newline (Python indent=2 + "\n").
        let text = std::fs::read_to_string(&output).expect("read back");
        assert!(text.ends_with("\n") && !text.ends_with("\n\n"), "{text:?}");
        assert!(text.contains("Chrome++ 构建版本"), "{text:?}");
    }

    #[test]
    fn product_boolean_preserved_for_unofficial() {
        let guard = temp_dir("product");
        let catalog = mini_catalog_file(&guard);
        let output = guard.path("build").join("brave.json");
        // brave_stable carries a bare "direct" provider with no url/path: the
        // Python script demands --url/--path for that shape.
        let err = write_selected(&catalog, "brave_stable", None, None, None, &output)
            .expect_err("direct placeholder");
        assert_eq!(
            err.to_string(),
            "Target 'brave_stable' needs --url or --path (no public auto-resolver)."
        );

        // With --url the write succeeds and product:false survives verbatim.
        write_selected(
            &catalog,
            "brave_stable",
            Some("https://example.invalid/b.exe"),
            None,
            None,
            &output,
        )
        .expect("write with url");
        let payload = read_output(&output);
        assert_eq!(payload["targets"]["brave_stable"]["product"], json!(false));
        assert_eq!(
            payload["targets"]["brave_stable"]["provider"],
            json!({"type": "direct", "verify_ssl": true, "url": "https://example.invalid/b.exe"})
        );
    }

    #[test]
    fn arch_override_replaces_catalog_value() {
        let guard = temp_dir("arch");
        let catalog = mini_catalog_file(&guard);
        let output = guard.path("build").join("arch.json");
        write_selected(
            &catalog,
            "chrome_stable",
            None,
            None,
            Some("arm64"),
            &output,
        )
        .expect("write");
        let payload = read_output(&output);
        assert_eq!(
            payload["targets"]["chrome_stable"]["architecture"],
            json!("arm64")
        );
        // Untouched fields stay as in the catalog.
        assert_eq!(
            payload["targets"]["chrome_stable"]["provider"]["type"],
            json!("google_omaha")
        );
    }

    #[test]
    fn url_and_path_overrides_replace_whole_provider_block() {
        let guard = temp_dir("overrides");
        let catalog = mini_catalog_file(&guard);
        let output = guard.path("build").join("ovr.json");
        write_selected(
            &catalog,
            "chrome_stable",
            Some("https://dl.example/ChromeSetup.exe"),
            Some("C:\\installers\\chrome.exe"),
            None,
            &output,
        )
        .expect("write");
        let payload = read_output(&output);
        // Python rebuilds the provider from scratch: google_omaha keys are gone.
        assert_eq!(
            payload["targets"]["chrome_stable"]["provider"],
            json!({
                "type": "direct",
                "verify_ssl": true,
                "url": "https://dl.example/ChromeSetup.exe",
                "path": "C:\\installers\\chrome.exe"
            })
        );
    }

    #[test]
    fn resolve_catalog_prefers_flag_then_env_then_cwd() {
        let guard = temp_dir("resolve");
        let catalog = mini_catalog_file(&guard);
        // Level 1: explicit builder dir.
        let dir = catalog.parent().unwrap().parent().unwrap();
        assert_eq!(
            resolve_catalog_path(Some(&dir.to_string_lossy())).expect("flag wins"),
            catalog
        );
        // Level 2: env var.
        // SAFETY: tests are single-threaded per var; std::env::set_var is
        // process-global, so run this probe last in the file order it appears.
        std::env::set_var("PORTABLE_BUILDER_DIR", &*dir.to_string_lossy());
        let resolved = resolve_catalog_path(None).expect("env wins");
        std::env::remove_var("PORTABLE_BUILDER_DIR");
        assert_eq!(resolved, catalog);
        // Fallback: no candidate contains the catalog -> last probed path named.
        let fallback = resolve_catalog_path(Some("Z:/definitely/not/here")).expect("fallback path");
        assert!(
            fallback.to_string_lossy().contains("not/here")
                || fallback.to_string_lossy().contains("not\\here")
        );
    }
}
