//! IronCowork core runtime.
//!
//! Phase 0 keeps the Rust agent engine, safety model, tool runtime, and
//! workspace retrieval stack while the product shell is being transformed into
//! a local-first desktop automation system.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────────┐
//! │                              Runtime Entry Layer                                 │
//! │  ┌──────────┐  ┌──────────┐  ┌─────────────┐                                     │
//! │  │   CLI    │  │   HTTP   │  │ Future UI   │                                     │
//! │  └────┬─────┘  └────┬─────┘  └──────┬──────┘                                     │
//! │       └─────────────┴───────────────┴─────────────────────────────────────────── │
//! └──────────────────────────────────┼──────────────────────────────────────────────┘
//!                                    ▼
//! ┌──────────────────────────────────────────────────────────────────────────────────┐
//! │                              Main Agent Loop                                      │
//! │  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐                      │
//! │  │ Message Router │──│  LLM Reasoning │──│ Action Executor│                      │
//! │  └────────────────┘  └───────┬────────┘  └───────┬────────┘                      │
//! │         ▲                    │                   │                               │
//! │         │         ┌──────────┴───────────────────┴──────────┐                    │
//! │         │         ▼                                         ▼                    │
//! │  ┌──────┴─────────────┐                         ┌───────────────────────┐        │
//! │  │   Safety Layer     │                         │    Self-Repair        │        │
//! │  │ - Input sanitizer  │                         │ - Stuck job detection │        │
//! │  │ - Injection defense│                         │ - Tool fixer          │        │
//! │  └────────────────────┘                         └───────────────────────┘        │
//! └──────────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Features
//!
//! - **Local-first runtime** - Embedded libSQL storage and direct local configuration
//! - **Parallel job execution** - Run multiple jobs with isolated contexts
//! - **Pluggable tools** - MCP, 3rd party services, dynamic tools
//! - **Self-repair** - Detect and fix stuck jobs and broken tools
//! - **Prompt injection defense** - Sanitize all external data
//! - **Continuous learning** - Improve estimates from historical data

pub mod agent;
pub mod api;
pub mod app;
pub mod boot_screen;
pub mod bootstrap;
pub mod channels;
pub mod cli;
pub mod config;
pub mod context;
pub mod db;
pub mod document_extraction;
pub mod desktop_runtime;
pub mod error;
pub mod estimation;
pub mod evaluation;
pub mod extensions;
pub mod file_archive_workflow;
pub mod history;
pub mod hooks;
#[cfg(feature = "import")]
pub mod import;
pub mod llm;
pub mod observability;
pub mod orchestrator;
pub mod pairing;
pub mod profile;
pub mod registry;
pub mod runtime_events;
pub mod safety;
pub mod sandbox;
pub mod secrets;
pub mod service;
pub mod settings;
pub mod skills;
pub mod task_runtime;
pub mod task_templates;
pub mod tenant;
pub mod timezone;
pub mod tools;
pub mod tracing_fmt;
pub mod util;
pub mod worker;
pub mod workspace;

#[cfg(test)]
pub mod testing;

pub use config::Config;
pub use error::{Error, Result};

/// Re-export commonly used types.
pub mod prelude {
    pub use crate::channels::{IncomingMessage, MessageStream};
    pub use crate::config::Config;
    pub use crate::context::{JobContext, JobState};
    pub use crate::error::{Error, Result};
    pub use crate::llm::LlmProvider;
    pub use crate::safety::{SanitizedOutput, Sanitizer};
    pub use crate::tools::{Tool, ToolOutput, ToolRegistry};
    pub use crate::workspace::{MemoryDocument, Workspace};
}
