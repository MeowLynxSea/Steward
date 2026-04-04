use std::process::Command;

use anyhow::{Context, bail};

pub async fn run_desktop_command() -> anyhow::Result<()> {
    let repo_root = std::env::current_dir().context("failed to resolve current directory")?;

    let frontend_status = Command::new("npm")
        .args(["--prefix", "ui", "run", "build"])
        .current_dir(&repo_root)
        .status()
        .context("failed to build desktop frontend assets")?;
    if !frontend_status.success() {
        bail!("frontend build failed");
    }

    // Warm the native shell in the same profile/features that `cargo tauri dev`
    // uses so the final launch phase can reuse incremental artifacts.
    let tauri_build_status = Command::new("cargo")
        .args(["build", "-p", "ironcowork-tauri", "--no-default-features"])
        .current_dir(&repo_root)
        .status()
        .context("failed to prebuild tauri desktop shell")?;
    if !tauri_build_status.success() {
        bail!("tauri desktop shell prebuild failed");
    }

    let status = Command::new("cargo")
        .args([
            "tauri",
            "dev",
            "--config",
            "src-tauri/tauri.conf.json",
            "--no-dev-server-wait",
        ])
        .current_dir(&repo_root)
        .status()
        .context("failed to launch tauri desktop shell")?;

    if !status.success() {
        bail!("tauri desktop shell exited with failure");
    }

    Ok(())
}
