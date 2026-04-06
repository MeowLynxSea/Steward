//! Channel manager for coordinating multiple ingress channels.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream;
use tokio::sync::{RwLock, mpsc};

use crate::channels::{
    Channel, IncomingMessage, MessageStream, MessageTransport, OutgoingResponse, StatusUpdate,
};
use crate::error::ChannelError;

pub struct ChannelManager {
    channels: Arc<RwLock<HashMap<String, Arc<dyn Channel>>>>,
    inject_tx: mpsc::Sender<IncomingMessage>,
    inject_rx: tokio::sync::Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
}

impl ChannelManager {
    pub fn new() -> Self {
        let (inject_tx, inject_rx) = mpsc::channel(64);
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            inject_tx,
            inject_rx: tokio::sync::Mutex::new(Some(inject_rx)),
        }
    }

    pub fn inject_sender(&self) -> mpsc::Sender<IncomingMessage> {
        self.inject_tx.clone()
    }

    pub async fn add(&self, channel: Box<dyn Channel>) {
        let name = channel.name().to_string();
        self.channels
            .write()
            .await
            .insert(name.clone(), Arc::from(channel));
        tracing::debug!("Added channel: {}", name);
    }

    pub async fn hot_add(&self, channel: Box<dyn Channel>) -> Result<(), ChannelError> {
        let name = channel.name().to_string();

        {
            let channels = self.channels.read().await;
            if let Some(existing) = channels.get(&name) {
                let _ = existing.shutdown().await;
            }
        }

        let stream = channel.start().await?;

        self.channels
            .write()
            .await
            .insert(name.clone(), Arc::from(channel));

        let tx = self.inject_tx.clone();
        tokio::spawn(async move {
            use futures::StreamExt;
            let mut stream = stream;
            while let Some(msg) = stream.next().await {
                if tx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        Ok(())
    }

    pub async fn start_all(&self) -> Result<MessageStream, ChannelError> {
        let channels = self.channels.read().await;
        let mut streams: Vec<MessageStream> = Vec::new();

        for channel in channels.values() {
            match channel.start().await {
                Ok(stream) => streams.push(stream),
                Err(error) => tracing::error!(%error, "Failed to start channel"),
            }
        }

        if let Some(inject_rx) = self.inject_rx.lock().await.take() {
            streams.push(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
                inject_rx,
            )));
        }

        if streams.is_empty() {
            return Err(ChannelError::StartupFailed {
                name: "all".to_string(),
                reason: "No channels started successfully".to_string(),
            });
        }

        Ok(Box::pin(stream::select_all(streams)))
    }

    pub async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(&msg.channel) {
            channel.respond(msg, response).await
        } else {
            Err(ChannelError::SendFailed {
                name: msg.channel.clone(),
                reason: "Channel not found".to_string(),
            })
        }
    }

    pub async fn send_status(
        &self,
        channel_name: &str,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(channel_name) {
            channel.send_status(status, metadata).await
        } else {
            Ok(())
        }
    }

    pub async fn broadcast(
        &self,
        channel_name: &str,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(channel_name) {
            channel.broadcast(user_id, response).await
        } else {
            Err(ChannelError::SendFailed {
                name: channel_name.to_string(),
                reason: "Channel not found".to_string(),
            })
        }
    }

    pub async fn shutdown_all(&self) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        for channel in channels.values() {
            let _ = channel.shutdown().await;
        }
        Ok(())
    }
}

impl Default for ChannelManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MessageTransport for ChannelManager {
    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Self::respond(self, msg, response).await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let channel_name = metadata
            .get("notify_channel")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        Self::send_status(self, channel_name, status, metadata).await
    }

    async fn broadcast(
        &self,
        channel_name: &str,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Self::broadcast(self, channel_name, user_id, response).await
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.shutdown_all().await
    }
}
