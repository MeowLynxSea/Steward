use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::BrainConfig;

/// A single slot in Working Memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemorySlot {
    pub node_id: Uuid,
    pub uri: String,
    pub title: String,
    pub content: String,
    pub relevance: f64,
    pub source: String,
    pub injection_depth: String,
    pub inserted_at: DateTime<Utc>,
    pub refresh_count: i32,
}

/// Working Memory state for a single namespace/session.
#[derive(Debug, Clone)]
pub struct WorkingMemoryState {
    pub slots: Vec<WorkingMemorySlot>,
    pub capacity: usize,
}

impl WorkingMemoryState {
    pub fn new(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            capacity,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Format WM slots as a prompt injection block.
    pub fn format_for_prompt(&self) -> String {
        if self.slots.is_empty() {
            return String::new();
        }
        let mut lines = vec![
            "============================================================".to_string(),
            "WORKING MEMORY — Active Contexts".to_string(),
            "============================================================".to_string(),
        ];
        for (i, slot) in self.slots.iter().enumerate() {
            lines.push(format!(
                "\n[{}] {}  (relevance: {:.2}, source: {})",
                i + 1,
                slot.uri,
                slot.relevance,
                slot.source
            ));
            lines.push(slot.content.clone());
        }
        lines.join("\n")
    }

    /// Serialize a snapshot of current slots for episodic memory.
    pub fn to_snapshot(&self) -> Vec<crate::memory::WmSnapshotEntry> {
        self.slots
            .iter()
            .map(|slot| crate::memory::WmSnapshotEntry {
                node_id: slot.node_id,
                uri: slot.uri.clone(),
                title: slot.title.clone(),
                relevance: slot.relevance,
            })
            .collect()
    }
}

/// Manages Working Memory pools per session/namespace.
#[derive(Clone)]
pub struct WorkingMemoryManager {
    states: Arc<std::sync::Mutex<HashMap<String, WorkingMemoryState>>>,
    config: BrainConfig,
}

impl WorkingMemoryManager {
    pub fn new(config: BrainConfig) -> Self {
        Self {
            states: Arc::new(std::sync::Mutex::new(HashMap::new())),
            config,
        }
    }

    fn state_key(owner_id: &str, agent_id: Option<Uuid>, session_id: &str) -> String {
        format!("{}:{}:{}", owner_id, agent_id.map(|u| u.to_string()).unwrap_or_default(), session_id)
    }

    /// Get or create the WM state for a session.
    pub fn get_state(&self, owner_id: &str, agent_id: Option<Uuid>, session_id: &str) -> WorkingMemoryState {
        let key = Self::state_key(owner_id, agent_id, session_id);
        let mut states = self.states.lock().unwrap_or_else(|e: std::sync::PoisonError<std::sync::MutexGuard<HashMap<String, WorkingMemoryState>>>| e.into_inner());
        states.get(&key).cloned().unwrap_or_else(|| {
            let state = WorkingMemoryState::new(self.config.wm_capacity);
            states.insert(key, state.clone());
            state
        })
    }

    /// Update WM with new candidates using competitive insertion.
    ///
    /// Candidates must beat the lowest slot by `wm_margin_threshold` to evict.
    /// Refreshing an existing slot gets a stickiness boost.
    pub fn update(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        session_id: &str,
        candidates: Vec<CandidateSlot>,
    ) -> Vec<BrainEvent> {
        let key = Self::state_key(owner_id, agent_id, session_id);
        let mut states = self.states.lock().unwrap_or_else(|e: std::sync::PoisonError<std::sync::MutexGuard<HashMap<String, WorkingMemoryState>>>| e.into_inner());
        let state = states.entry(key.clone()).or_insert_with(|| {
            WorkingMemoryState::new(self.config.wm_capacity)
        });

        let mut events = Vec::new();
        for candidate in candidates {
            let existing_index = state
                .slots
                .iter()
                .position(|slot| slot.node_id == candidate.node_id);

            if let Some(index) = existing_index {
                // Refresh existing slot
                let slot = &mut state.slots[index];
                slot.relevance = candidate.relevance + self.config.wm_stickiness_boost;
                slot.refresh_count += 1;
                slot.content = candidate.content;
                events.push(BrainEvent::WmRefreshed {
                    slot_index: index,
                    node_id: candidate.node_id,
                    uri: candidate.uri.clone(),
                    relevance: slot.relevance,
                });
            } else if state.slots.len() < state.capacity {
                // Simple insert if room available
                let injection_depth = if candidate.relevance > self.config.wm_full_injection_threshold {
                    "full"
                } else {
                    "summary"
                };
                state.slots.push(WorkingMemorySlot {
                    node_id: candidate.node_id,
                    uri: candidate.uri.clone(),
                    title: candidate.title.clone(),
                    content: candidate.content,
                    relevance: candidate.relevance,
                    source: candidate.source.clone(),
                    injection_depth: injection_depth.to_string(),
                    inserted_at: Utc::now(),
                    refresh_count: 0,
                });
                events.push(BrainEvent::WmInserted {
                    slot_index: state.slots.len() - 1,
                    node_id: candidate.node_id,
                    uri: candidate.uri,
                    relevance: candidate.relevance,
                });
            } else {
                // Competitive eviction
                let min_index = state
                    .slots
                    .iter()
                    .enumerate()
                    .min_by(|a, b| a.1.relevance.partial_cmp(&b.1.relevance).unwrap())
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let min_relevance = state.slots[min_index].relevance;

                if candidate.relevance >= min_relevance + self.config.wm_margin_threshold {
                    let evicted = state.slots.remove(min_index);
                    events.push(BrainEvent::WmEvicted {
                        slot_index: min_index,
                        node_id: evicted.node_id,
                        uri: evicted.uri,
                    });

                    let injection_depth = if candidate.relevance > self.config.wm_full_injection_threshold {
                        "full"
                    } else {
                        "summary"
                    };
                    state.slots.push(WorkingMemorySlot {
                        node_id: candidate.node_id,
                        uri: candidate.uri.clone(),
                        title: candidate.title.clone(),
                        content: candidate.content,
                        relevance: candidate.relevance,
                        source: candidate.source.clone(),
                        injection_depth: injection_depth.to_string(),
                        inserted_at: Utc::now(),
                        refresh_count: 0,
                    });
                    events.push(BrainEvent::WmInserted {
                        slot_index: state.slots.len() - 1,
                        node_id: candidate.node_id,
                        uri: candidate.uri,
                        relevance: candidate.relevance,
                    });
                }
            }
        }

        // Sort by relevance descending for stable ordering
        state.slots.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap());
        events
    }

    /// Manually inject a node into WM (e.g. session_start boot nodes).
    pub fn inject(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        session_id: &str,
        candidate: CandidateSlot,
    ) -> Vec<BrainEvent> {
        let mut boosted = candidate;
        boosted.relevance += self.config.wm_manual_boost;
        self.update(owner_id, agent_id, session_id, vec![boosted])
    }

    /// Clear WM for a session.
    pub fn clear(&self, owner_id: &str, agent_id: Option<Uuid>, session_id: &str) {
        let key = Self::state_key(owner_id, agent_id, session_id);
        let mut states = self.states.lock().unwrap_or_else(|e: std::sync::PoisonError<std::sync::MutexGuard<HashMap<String, WorkingMemoryState>>>| e.into_inner());
        states.remove(&key);
    }
}

use crate::brain::events::BrainEvent;

/// A candidate for Working Memory insertion.
#[derive(Debug, Clone)]
pub struct CandidateSlot {
    pub node_id: Uuid,
    pub uri: String,
    pub title: String,
    pub content: String,
    pub relevance: f64,
    pub source: String,
}
