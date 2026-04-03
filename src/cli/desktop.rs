use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context, bail};

struct LocalApiProcess {
    child: Child,
}

impl LocalApiProcess {
    fn spawn(current_exe: &Path, port: u16) -> anyhow::Result<Self> {
        let child = Command::new(current_exe)
            .args(["api", "serve", "--port", &port.to_string()])
            .stdin(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start local api on port {port}"))?;
        Ok(Self { child })
    }
}

impl Drop for LocalApiProcess {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub async fn run_desktop_command() -> anyhow::Result<()> {
    let repo_root = std::env::current_dir().context("failed to resolve current directory")?;
    let current_exe = std::env::current_exe().context("failed to resolve current executable")?;
    let api_port = crate::api::DEFAULT_API_PORT;
    let api_base = format!("http://127.0.0.1:{api_port}");

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

    let _api = LocalApiProcess::spawn(&current_exe, api_port)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .context("failed to create readiness probe client")?;

    let health_url = format!("{api_base}/api/v0/health");
    let mut ready = false;
    for _ in 0..120 {
        if let Ok(response) = client.get(&health_url).send().await
            && response.status().is_success()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    if !ready {
        bail!("local api failed to become ready on {api_base}");
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
