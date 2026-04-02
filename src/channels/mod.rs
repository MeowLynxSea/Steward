//! Message ingress primitives.

mod channel;

pub use channel::{
    AttachmentKind, IncomingAttachment, IncomingMessage, MessageTransport,
    MessageStream, OutgoingResponse, StatusUpdate, ToolDecision, routing_target_from_metadata,
};
