//! Brain-inspired cognitive memory system for Steward.
//!
//! This module implements a four-layer memory architecture inspired by
//! class-brain cognitive models:
//! - Working Memory (competitive limited-capacity cache)
//! - Activation Engine (multi-signal scoring for memory retrieval)
//! - Associative Network (Hebbian-learned weighted edges)
//! - Episodic Memory (activation traces with WM snapshots)
//! - Dream Engine (periodic consolidation: decay + replay reinforcement)

pub mod activation_engine;
pub mod associative;
pub mod dream_engine;
pub mod episodic;
pub mod events;
pub mod working_memory;

pub use activation_engine::ActivationEngine;
pub use associative::AssociativeNetwork;
pub use dream_engine::{DreamEngine, DreamReport};
pub use episodic::EpisodicMemory;
pub use working_memory::{WorkingMemoryManager, WorkingMemorySlot, WorkingMemoryState};
