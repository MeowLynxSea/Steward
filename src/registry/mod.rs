//! Extension registry: metadata catalog for tools and MCP servers.
//!
//! The registry provides a central index of all available extensions (WASM tools
//! and MCP servers) with their source locations, build artifacts, authentication
//! requirements, and grouping via bundles.
//!
//! ```text
//! registry/
//! ├── tools/          <- One JSON manifest per tool
//! ├── mcp-servers/    <- One JSON manifest per MCP server
//! └── _bundles.json   <- Bundle definitions (google, messaging, default)
//! ```

pub mod artifacts;
pub mod catalog;
pub mod embedded;
pub mod installer;
pub mod manifest;

pub use catalog::{RegistryCatalog, RegistryError};
pub use installer::RegistryInstaller;
pub use manifest::{
    ArtifactSpec, AuthSummary, BundleDefinition, BundlesFile, ExtensionManifest, ManifestKind,
    SourceSpec,
};
