use std::path::PathBuf;

use crate::bootstrap::steward_base_dir;
use crate::error::ConfigError;
use crate::settings::Settings;

/// Desktop transport configuration.
#[derive(Debug, Clone)]
pub struct ChannelsConfig {
    pub desktop: DesktopConfig,
    pub wasm_channels: WasmChannelsConfig,
}

/// The only supported primary product transport.
#[derive(Debug, Clone)]
pub struct DesktopConfig {
    pub tauri_ipc: bool,
}

#[derive(Debug, Clone)]
pub struct WasmChannelsConfig {
    pub enabled: bool,
    pub dir: PathBuf,
}

fn default_wasm_channels_dir() -> PathBuf {
    steward_base_dir().join("channels")
}

impl ChannelsConfig {
    pub(crate) fn resolve(settings: &Settings, _owner_id: &str) -> Result<Self, ConfigError> {
        Ok(Self {
            desktop: DesktopConfig {
                tauri_ipc: settings.channels.tauri_ipc,
            },
            wasm_channels: WasmChannelsConfig {
                enabled: settings.channels.wasm_channels_enabled,
                dir: settings
                    .channels
                    .wasm_channels_dir
                    .clone()
                    .unwrap_or_else(default_wasm_channels_dir),
            },
        })
    }
}
