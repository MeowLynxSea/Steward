//! Shared runtime event broadcasting primitives.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use steward_common::AppEvent;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

const MAX_CONNECTIONS: u64 = 100;

/// Trait for runtime event emitters (SSE or Tauri).
///
/// This trait abstracts over different event broadcasting mechanisms,
/// allowing the codebase to work with both SSE (Server-Sent Events)
/// and Tauri native events without changing the call sites.
pub trait RuntimeEventEmitter: Send + Sync {
    /// Emit an event for a specific user.
    fn emit_for_user(&self, user_id: &str, event: AppEvent);
}


#[derive(Debug, Clone)]
pub(crate) struct ScopedEvent {
    pub(crate) user_id: Option<String>,
    pub(crate) event: AppEvent,
}

pub struct SseManager {
    tx: broadcast::Sender<ScopedEvent>,
    connection_count: Arc<AtomicU64>,
    max_connections: u64,
}

impl SseManager {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            tx,
            connection_count: Arc::new(AtomicU64::new(0)),
            max_connections: MAX_CONNECTIONS,
        }
    }

    pub fn broadcast(&self, event: AppEvent) {
        let _ = self.tx.send(ScopedEvent {
            user_id: None,
            event,
        });
    }

    pub fn broadcast_for_user(&self, user_id: &str, event: AppEvent) {
        let _ = self.tx.send(ScopedEvent {
            user_id: Some(user_id.to_string()),
            event,
        });
    }

    pub fn connection_count(&self) -> u64 {
        self.connection_count.load(Ordering::Relaxed)
    }

    pub fn subscribe_raw(
        &self,
        user_id: Option<String>,
    ) -> Option<impl Stream<Item = AppEvent> + Send + 'static + use<>> {
        let counter = Arc::clone(&self.connection_count);
        let max = self.max_connections;
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                if current < max {
                    Some(current + 1)
                } else {
                    None
                }
            })
            .ok()?;
        let rx = self.tx.subscribe();

        let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
            Ok(scoped) => match (&user_id, &scoped.user_id) {
                (_, None) => Some(scoped.event),
                (None, _) => Some(scoped.event),
                (Some(sub), Some(ev)) if sub == ev => Some(scoped.event),
                _ => None,
            },
            Err(_) => None,
        });

        Some(CountedStream {
            inner: stream,
            counter,
        })
    }

    pub fn subscribe(
        &self,
        user_id: Option<String>,
    ) -> Option<Sse<impl Stream<Item = Result<Event, Infallible>> + Send + 'static + use<>>> {
        let counter = Arc::clone(&self.connection_count);
        let max = self.max_connections;
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                if current < max {
                    Some(current + 1)
                } else {
                    None
                }
            })
            .ok()?;
        let rx = self.tx.subscribe();

        let stream = BroadcastStream::new(rx)
            .filter_map(move |result| match result {
                Ok(scoped) => match (&user_id, &scoped.user_id) {
                    (_, None) => Some(scoped.event),
                    (None, _) => Some(scoped.event),
                    (Some(sub), Some(ev)) if sub == ev => Some(scoped.event),
                    _ => None,
                },
                Err(_) => None,
            })
            .filter_map(|event| {
                let data = match serde_json::to_string(&event) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("Failed to serialize SSE event: {}", e);
                        return None;
                    }
                };
                Some(Ok(Event::default().event(event.event_type()).data(data)))
            });

        Some(
            Sse::new(CountedStream {
                inner: stream,
                counter,
            })
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(30)).text("")),
        )
    }
}

impl Default for SseManager {
    fn default() -> Self {
        Self::new()
    }
}

struct CountedStream<S> {
    inner: S,
    counter: Arc<AtomicU64>,
}

impl<S: Stream + Unpin> Stream for CountedStream<S> {
    type Item = S::Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<S> Drop for CountedStream<S> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}
