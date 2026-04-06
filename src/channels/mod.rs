//! Message ingress primitives.

mod channel;
mod manager;
pub mod wasm;

pub use channel::{
    AttachmentKind, Channel, ChannelSecretUpdater, IncomingAttachment, IncomingMessage,
    MessageStream, MessageTransport, OutgoingResponse, StatusUpdate, ToolDecision,
    routing_target_from_metadata,
};
pub use manager::ChannelManager;
