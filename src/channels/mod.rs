//! Message ingress primitives.

mod channel;

pub use channel::{
    AttachmentKind, IncomingAttachment, IncomingMessage, MessageStream, MessageTransport,
    OutgoingResponse, StatusUpdate, ToolDecision, routing_target_from_metadata,
};
