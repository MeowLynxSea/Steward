//! Safety layer for prompt injection defense.
//!
//! This module re-exports everything from the `steward_safety` crate,
//! keeping `crate::safety::*` imports working throughout the codebase.

pub use steward_safety::*;
