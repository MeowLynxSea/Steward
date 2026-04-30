use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Real-time event broadcast from the brain subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BrainEvent {
    /// A node was activated by a query.
    NodeActivated {
        node_id: Uuid,
        uri: String,
        title: String,
        activation_strength: f64,
        source: String,
    },
    /// Working Memory slot inserted.
    WmInserted {
        slot_index: usize,
        node_id: Uuid,
        uri: String,
        relevance: f64,
    },
    /// Working Memory slot refreshed (already present, relevance updated).
    WmRefreshed {
        slot_index: usize,
        node_id: Uuid,
        uri: String,
        relevance: f64,
    },
    /// Working Memory slot evicted.
    WmEvicted {
        slot_index: usize,
        node_id: Uuid,
        uri: String,
    },
    /// Associative edge was reinforced.
    EdgeReinforced {
        source_node_id: Uuid,
        target_node_id: Uuid,
        new_weight: f64,
    },
}
