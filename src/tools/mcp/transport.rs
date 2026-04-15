//! Shared MCP transport trait and JSON-RPC framing helpers.
//!
//! Provides the [`McpTransport`] trait that all MCP transports implement,
//! plus framing helpers for newline-delimited JSON-RPC over byte streams
//! (used by stdio and unix socket transports).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::task::JoinHandle;

use crate::tools::mcp::protocol::{McpRequest, McpResponse};
use crate::tools::tool::ToolError;

/// A server-originated MCP notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// A server-originated MCP request that requires a response from Steward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// Inbound server-originated MCP traffic.
#[derive(Debug, Clone)]
pub enum McpInboundMessage {
    Notification(McpNotification),
    Request(McpServerRequest),
}

#[derive(Debug, Clone)]
pub(crate) enum ParsedJsonRpcMessage {
    Response(McpResponse),
    Notification(McpNotification),
    Request(McpServerRequest),
}

/// Trait for sending JSON-RPC requests to an MCP server and receiving responses.
///
/// Implementations handle the underlying transport (HTTP, stdio, unix socket, etc.).
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a request and wait for the corresponding response.
    ///
    /// `headers` are used by HTTP-based transports (e.g., `Mcp-Session-Id`);
    /// stream-based transports may ignore them.
    async fn send(
        &self,
        request: &McpRequest,
        headers: &HashMap<String, String>,
    ) -> Result<McpResponse, ToolError>;

    /// Shut down the transport, releasing any resources (child processes, connections).
    async fn shutdown(&self) -> Result<(), ToolError>;

    /// Whether this transport supports HTTP-specific features like session headers.
    fn supports_http_features(&self) -> bool {
        false
    }

    /// Subscribe to server-originated notifications and requests.
    fn subscribe_inbound(&self) -> Option<broadcast::Receiver<McpInboundMessage>> {
        None
    }

    /// Send a raw JSON-RPC response or notification back to the server.
    async fn send_jsonrpc_message(
        &self,
        _message: &serde_json::Value,
        _headers: &HashMap<String, String>,
    ) -> Result<(), ToolError> {
        Err(ToolError::ExternalService(
            "MCP transport does not support outbound raw JSON-RPC messages".to_string(),
        ))
    }
}

/// Serialize an [`McpRequest`] as a single JSON line and write it to `writer`.
///
/// The line is terminated with `\n` and the writer is flushed.
pub async fn write_jsonrpc_line(
    writer: &mut (impl AsyncWrite + Unpin),
    request: &McpRequest,
) -> Result<(), ToolError> {
    let value = serde_json::to_value(request).map_err(|e| {
        ToolError::ExternalService(format!("Failed to serialize JSON-RPC request: {e}"))
    })?;
    write_jsonrpc_value_line(writer, &value).await
}

/// Serialize a raw JSON-RPC value as a single line and write it to `writer`.
pub async fn write_jsonrpc_value_line(
    writer: &mut (impl AsyncWrite + Unpin),
    message: &serde_json::Value,
) -> Result<(), ToolError> {
    let json = serde_json::to_string(message).map_err(|e| {
        ToolError::ExternalService(format!("Failed to serialize JSON-RPC message: {e}"))
    })?;

    writer.write_all(json.as_bytes()).await.map_err(|e| {
        ToolError::ExternalService(format!("Failed to write JSON-RPC message: {e}"))
    })?;

    writer
        .write_all(b"\n")
        .await
        .map_err(|e| ToolError::ExternalService(format!("Failed to write newline: {e}")))?;

    writer
        .flush()
        .await
        .map_err(|e| ToolError::ExternalService(format!("Failed to flush JSON-RPC writer: {e}")))?;

    Ok(())
}

/// Parse a JSON value into a response, notification, or server-originated request.
pub(crate) fn parse_jsonrpc_message(
    value: serde_json::Value,
) -> Result<ParsedJsonRpcMessage, ToolError> {
    let has_method = value.get("method").and_then(|v| v.as_str()).is_some();
    let has_id = value.get("id").is_some_and(|id| !id.is_null());

    if has_method && has_id {
        let request: McpServerRequest = serde_json::from_value(value).map_err(|e| {
            ToolError::ExternalService(format!("Failed to parse server-originated request: {e}"))
        })?;
        return Ok(ParsedJsonRpcMessage::Request(request));
    }

    if has_method {
        let notification: McpNotification = serde_json::from_value(value).map_err(|e| {
            ToolError::ExternalService(format!("Failed to parse notification: {e}"))
        })?;
        return Ok(ParsedJsonRpcMessage::Notification(notification));
    }

    let response: McpResponse = serde_json::from_value(value).map_err(|e| {
        ToolError::ExternalService(format!("Failed to parse JSON-RPC response: {e}"))
    })?;
    Ok(ParsedJsonRpcMessage::Response(response))
}

/// Spawn a background task that reads newline-delimited JSON-RPC messages from
/// `reader`, dispatching matched responses to `pending` and publishing
/// notifications / server requests to `inbound`.
pub fn spawn_jsonrpc_reader<R: AsyncBufRead + Unpin + Send + 'static>(
    reader: R,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>>,
    server_name: String,
    inbound: Option<broadcast::Sender<McpInboundMessage>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let value = match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(value) => value,
                Err(e) => {
                    let preview: String = line.chars().take(200).collect();
                    tracing::debug!(
                        "[{}] Failed to parse JSON-RPC message: {} — line: {}{}",
                        server_name,
                        e,
                        preview,
                        if line.len() > 200 { "…" } else { "" }
                    );
                    continue;
                }
            };

            match parse_jsonrpc_message(value) {
                Ok(ParsedJsonRpcMessage::Response(response)) => {
                    let Some(id) = response.id else {
                        tracing::debug!(
                            "[{}] Received JSON-RPC response without id, skipping dispatch",
                            server_name
                        );
                        continue;
                    };

                    let mut map = pending.lock().await;
                    if let Some(tx) = map.remove(&id) {
                        let _ = tx.send(response);
                    } else {
                        tracing::debug!(
                            "[{}] Received response for unknown request id {}",
                            server_name,
                            id
                        );
                    }
                }
                Ok(ParsedJsonRpcMessage::Notification(notification)) => {
                    tracing::debug!(
                        "[{}] Received MCP notification '{}'",
                        server_name,
                        notification.method
                    );
                    if let Some(inbound_tx) = &inbound {
                        let _ = inbound_tx.send(McpInboundMessage::Notification(notification));
                    }
                }
                Ok(ParsedJsonRpcMessage::Request(request)) => {
                    tracing::debug!(
                        "[{}] Received server-originated MCP request '{}'",
                        server_name,
                        request.method
                    );
                    if let Some(inbound_tx) = &inbound {
                        let _ = inbound_tx.send(McpInboundMessage::Request(request));
                    }
                }
                Err(e) => {
                    tracing::debug!("[{}] Failed to route JSON-RPC message: {}", server_name, e);
                }
            }
        }

        tracing::debug!("[{}] JSON-RPC reader finished", server_name);
    })
}

/// Send a JSON-RPC request over a stream-based transport (stdio / unix socket).
///
/// Handles notification fire-and-forget, pending response registration,
/// write, timeout, and cleanup. Used by both [`StdioMcpTransport`] and
/// [`UnixMcpTransport`] to avoid duplicating the send logic.
pub(crate) async fn stream_transport_send<W: AsyncWrite + Unpin>(
    writer: &Mutex<W>,
    pending: &Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>,
    request: &McpRequest,
    server_name: &str,
    timeout_duration: std::time::Duration,
) -> Result<McpResponse, ToolError> {
    if request.id.is_none() {
        let mut w = writer.lock().await;
        write_jsonrpc_line(&mut *w, request).await?;
        return Ok(McpResponse {
            jsonrpc: "2.0".to_string(),
            id: None,
            result: None,
            error: None,
        });
    }

    let id = request.id.unwrap_or(0);
    let (tx, rx) = oneshot::channel();

    {
        let mut map = pending.lock().await;
        map.insert(id, tx);
    }

    {
        let mut w = writer.lock().await;
        if let Err(e) = write_jsonrpc_line(&mut *w, request).await {
            let mut map = pending.lock().await;
            map.remove(&id);
            return Err(e);
        }
    }

    match tokio::time::timeout(timeout_duration, rx).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(_)) => {
            let mut map = pending.lock().await;
            map.remove(&id);
            Err(ToolError::ExternalService(format!(
                "[{}] MCP server closed connection before responding to request {:?}",
                server_name, request.id
            )))
        }
        Err(_) => {
            let mut map = pending.lock().await;
            map.remove(&id);
            Err(ToolError::ExternalService(format!(
                "[{}] Timeout waiting for response to request {:?} after {:?}",
                server_name, request.id, timeout_duration
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_write_jsonrpc_line_serializes_and_flushes() {
        let request = McpRequest {
            jsonrpc: "2.0".into(),
            id: Some(1),
            method: "test/method".into(),
            params: None,
        };

        let mut buf = Vec::new();
        write_jsonrpc_line(&mut buf, &request)
            .await
            .expect("write should succeed");

        let written = String::from_utf8(buf).expect("should be valid UTF-8");
        assert!(written.ends_with('\n'));

        let parsed: serde_json::Value =
            serde_json::from_str(written.trim()).expect("should be valid JSON");
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["method"], "test/method");
    }

    #[tokio::test]
    async fn test_spawn_jsonrpc_reader_dispatches_response() {
        let response = McpResponse {
            jsonrpc: "2.0".into(),
            id: Some(42),
            result: Some(serde_json::json!({"tools": []})),
            error: None,
        };
        let line = format!("{}\n", serde_json::to_string(&response).unwrap());

        let reader = std::io::Cursor::new(line.into_bytes());
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let (tx, rx) = oneshot::channel();
        {
            let mut map = pending.lock().await;
            map.insert(42, tx);
        }

        let handle = spawn_jsonrpc_reader(reader, pending.clone(), "test".into(), None);

        let resp = rx.await.expect("should receive response");
        assert_eq!(resp.id, Some(42));
        assert!(resp.result.is_some());

        handle.await.expect("reader task should finish");
    }

    #[tokio::test]
    async fn test_spawn_jsonrpc_reader_skips_invalid_lines() {
        let input = "this is not json\n{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":null}\n";
        let reader = std::io::Cursor::new(input.as_bytes().to_vec());
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let (tx, rx) = oneshot::channel();
        {
            let mut map = pending.lock().await;
            map.insert(7, tx);
        }

        let handle = spawn_jsonrpc_reader(reader, pending.clone(), "test".into(), None);

        let resp = rx
            .await
            .expect("should receive response despite earlier invalid line");
        assert_eq!(resp.id, Some(7));

        handle.await.expect("reader task should finish");
    }

    #[tokio::test]
    async fn test_notification_does_not_resolve_pending_id_zero() {
        let notification = r#"{"jsonrpc":"2.0","method":"notifications/progress","params":{}}"#;
        let real_response = r#"{"jsonrpc":"2.0","id":0,"result":{"ok":true}}"#;
        let input = format!("{notification}\n{real_response}\n");

        let reader = std::io::Cursor::new(input.into_bytes());
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let (tx, rx) = oneshot::channel();
        {
            let mut map = pending.lock().await;
            map.insert(0, tx);
        }

        let handle = spawn_jsonrpc_reader(reader, pending.clone(), "test".into(), None);

        let resp = rx.await.expect("should receive the real id=0 response");
        assert_eq!(resp.id, Some(0));
        assert!(resp.result.is_some());

        handle.await.expect("reader task should finish");
    }

    #[tokio::test]
    async fn test_spawn_jsonrpc_reader_broadcasts_notifications_and_requests() {
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"roots-1\",\"method\":\"roots/list\",\"params\":{}}\n"
        );
        let reader = std::io::Cursor::new(input.as_bytes().to_vec());
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<McpResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (tx, mut rx) = broadcast::channel(8);

        let handle = spawn_jsonrpc_reader(reader, pending, "test".into(), Some(tx));

        match rx.recv().await.expect("notification") {
            McpInboundMessage::Notification(notification) => {
                assert_eq!(notification.method, "notifications/tools/list_changed");
            }
            other => panic!("expected notification, got {other:?}"),
        }

        match rx.recv().await.expect("request") {
            McpInboundMessage::Request(request) => {
                assert_eq!(request.method, "roots/list");
                assert_eq!(request.id, serde_json::json!("roots-1"));
            }
            other => panic!("expected request, got {other:?}"),
        }

        handle.await.expect("reader task should finish");
    }
}
