//! Session manager for single-user desktop conversation handling.
//!
//! Manages a single session with multiple threads, mapping external thread
//! IDs to internal UUIDs and managing undo state for each thread.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::agent::session::Session;
use crate::agent::undo::UndoManager;
use crate::hooks::HookRegistry;

/// Key for mapping external thread IDs to internal ones.
#[derive(Clone, Hash, Eq, PartialEq)]
struct ThreadKey {
    external_thread_id: Option<String>,
}

/// Manages a single session with multiple threads for the desktop user.
pub struct SessionManager {
    /// The single session for this desktop instance.
    session: RwLock<Option<Arc<Mutex<Session>>>>,
    /// Maps thread keys to internal thread UUIDs.
    thread_map: RwLock<HashMap<ThreadKey, Uuid>>,
    /// Undo managers per thread.
    undo_managers: RwLock<HashMap<Uuid, Arc<Mutex<UndoManager>>>>,
    hooks: Option<Arc<HookRegistry>>,
}

impl SessionManager {
    /// Create a new single-user session manager.
    pub fn new() -> Self {
        Self {
            session: RwLock::new(None),
            thread_map: RwLock::new(HashMap::new()),
            undo_managers: RwLock::new(HashMap::new()),
            hooks: None,
        }
    }

    /// Attach a hook registry for session lifecycle events.
    pub fn with_hooks(mut self, hooks: Arc<HookRegistry>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Get or create the single session.
    pub async fn get_session(&self) -> Arc<Mutex<Session>> {
        // Fast path: session already exists
        {
            if let Some(ref session) = *self.session.read().await {
                return Arc::clone(session);
            }
        }

        // Slow path: create session
        let mut guard = self.session.write().await;
        // Double-check after acquiring write lock
        if let Some(ref session) = *guard {
            return Arc::clone(session);
        }

        let new_session = Arc::new(Mutex::new(Session::new("default")));
        let session_id = new_session.lock().await.id.to_string();
        *guard = Some(Arc::clone(&new_session));

        // Fire OnSessionStart hook (fire-and-forget)
        if let Some(ref hooks) = self.hooks {
            let hooks = hooks.clone();
            tokio::spawn(async move {
                use crate::hooks::HookEvent;
                let event = HookEvent::SessionStart {
                    user_id: "default".to_string(),
                    session_id,
                };
                if let Err(e) = hooks.run(&event).await {
                    tracing::warn!("OnSessionStart hook error: {}", e);
                }
            });
        }

        new_session
    }

    /// Resolve an external thread ID to an internal thread.
    ///
    /// Returns the session and thread ID. Creates a new thread if it doesn't exist.
    pub async fn resolve_thread(
        &self,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        self.resolve_thread_with_parsed_uuid(external_thread_id, None)
            .await
    }

    /// Like [`resolve_thread`](Self::resolve_thread), but accepts a pre-parsed
    /// UUID to skip redundant parsing when the caller has already validated
    /// the external thread ID as a UUID.
    pub async fn resolve_thread_with_parsed_uuid(
        &self,
        external_thread_id: Option<&str>,
        parsed_uuid: Option<Uuid>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        let session = self.get_session().await;

        let key = ThreadKey {
            external_thread_id: external_thread_id.map(String::from),
        };

        // Use pre-parsed UUID if available, otherwise parse from string.
        let ext_uuid = parsed_uuid
            .or_else(|| external_thread_id.and_then(|ext_tid| Uuid::parse_str(ext_tid).ok()));

        // Validate that parsed_uuid (if provided) is consistent with external_thread_id.
        #[cfg(debug_assertions)]
        if let (Some(parsed), Some(ext_tid)) = (&parsed_uuid, external_thread_id) {
            debug_assert_eq!(
                Uuid::parse_str(ext_tid).ok().as_ref(),
                Some(parsed),
                "parsed_uuid must be the parsed form of external_thread_id"
            );
        }

        // Single read lock for both the key lookup and UUID adoption check
        let adoptable_uuid = {
            let thread_map = self.thread_map.read().await;

            // Fast path: exact key match
            if let Some(&thread_id) = thread_map.get(&key) {
                let sess = session.lock().await;
                if sess.threads.contains_key(&thread_id) {
                    return (Arc::clone(&session), thread_id);
                }
            }

            // UUID adoption: if external_thread_id is a valid UUID not mapped elsewhere,
            // it may be a thread created by chat_new_thread_handler or hydrated from DB.
            if external_thread_id.is_some() {
                ext_uuid.filter(|&uuid| !thread_map.values().any(|&v| v == uuid))
            } else {
                None
            }
        }; // Single read lock dropped here

        // If we found an adoptable UUID, verify it exists in session
        if let Some(ext_uuid) = adoptable_uuid {
            let sess = session.lock().await;
            if sess.threads.contains_key(&ext_uuid) {
                drop(sess);

                let mut thread_map = self.thread_map.write().await;
                if !thread_map.values().any(|&v| v == ext_uuid) {
                    thread_map.insert(key, ext_uuid);
                    drop(thread_map);
                    let mut undo_managers = self.undo_managers.write().await;
                    undo_managers
                        .entry(ext_uuid)
                        .or_insert_with(|| Arc::new(Mutex::new(UndoManager::new())));
                    return (session, ext_uuid);
                }
            }
        }

        // Create new thread
        let thread_id = {
            let mut sess = session.lock().await;
            let thread = sess.create_thread();
            thread.id
        };

        // Store mapping
        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.insert(key, thread_id);
        }

        // Create undo manager for thread
        {
            let mut undo_managers = self.undo_managers.write().await;
            undo_managers.insert(thread_id, Arc::new(Mutex::new(UndoManager::new())));
        }

        (session, thread_id)
    }

    /// Create a thread with an explicit UUID and bind it to an external thread key.
    ///
    /// Used by the local HTTP API so browser and desktop clients can treat the
    /// returned UUID as both the persisted conversation ID and the runtime thread ID.
    pub async fn create_bound_thread(
        &self,
        external_thread_id: &str,
        thread_id: Uuid,
    ) -> Arc<Mutex<Session>> {
        let session = self.get_session().await;
        {
            let mut sess = session.lock().await;
            let session_id = sess.id;
            sess.active_thread = Some(thread_id);
            sess.last_active_at = chrono::Utc::now();
            sess.threads
                .entry(thread_id)
                .or_insert_with(|| crate::agent::session::Thread::with_id(thread_id, session_id));
        }

        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.insert(
                ThreadKey {
                    external_thread_id: Some(external_thread_id.to_string()),
                },
                thread_id,
            );
        }

        {
            let mut undo_managers = self.undo_managers.write().await;
            undo_managers
                .entry(thread_id)
                .or_insert_with(|| Arc::new(Mutex::new(UndoManager::new())));
        }

        session
    }

    /// Register a hydrated thread so subsequent `resolve_thread` calls find it.
    pub async fn register_thread(
        &self,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        let key = ThreadKey {
            external_thread_id: Some(thread_id.to_string()),
        };

        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.insert(key, thread_id);
        }

        {
            let mut undo_managers = self.undo_managers.write().await;
            undo_managers
                .entry(thread_id)
                .or_insert_with(|| Arc::new(Mutex::new(UndoManager::new())));
        }

        // Ensure the session is tracked as the single session
        {
            let mut guard = self.session.write().await;
            *guard = Some(session);
        }
    }

    /// Get undo manager for a thread.
    pub async fn get_undo_manager(&self, thread_id: Uuid) -> Arc<Mutex<UndoManager>> {
        // Fast path
        {
            let managers = self.undo_managers.read().await;
            if let Some(mgr) = managers.get(&thread_id) {
                return Arc::clone(mgr);
            }
        }

        // Create if missing
        let mut managers = self.undo_managers.write().await;
        if let Some(mgr) = managers.get(&thread_id) {
            return Arc::clone(mgr);
        }

        let mgr = Arc::new(Mutex::new(UndoManager::new()));
        managers.insert(thread_id, Arc::clone(&mgr));
        mgr
    }

    /// Get the current session if it exists.
    pub async fn get_current_session(&self) -> Option<Arc<Mutex<Session>>> {
        self.session.read().await.clone()
    }

    /// Prune threads that have been idle for longer than the given duration.
    ///
    /// Returns the number of threads pruned.
    pub async fn prune_stale_threads(&self, max_idle: std::time::Duration) -> usize {
        let cutoff = chrono::Utc::now() - chrono::TimeDelta::seconds(max_idle.as_secs() as i64);

        let session = self.get_session().await;
        let stale_thread_ids: Vec<Uuid> = {
            let sess = session.lock().await;
            sess.threads
                .values()
                .filter(|t| t.updated_at < cutoff)
                .map(|t| t.id)
                .collect()
        };

        if stale_thread_ids.is_empty() {
            return 0;
        }

        // Fire OnSessionEnd hooks for stale threads (fire-and-forget)
        if let Some(ref hooks) = self.hooks {
            let hooks = hooks.clone();
            let session_id = session.lock().await.id.to_string();
            let stale_ids = stale_thread_ids.clone();
            tokio::spawn(async move {
                use crate::hooks::HookEvent;
                for _thread_id in &stale_ids {
                    let event = HookEvent::SessionEnd {
                        user_id: "default".to_string(),
                        session_id: session_id.clone(),
                    };
                    if let Err(e) = hooks.run(&event).await {
                        tracing::warn!("OnSessionEnd hook error: {}", e);
                    }
                }
            });
        }

        // Remove stale threads from session
        {
            let mut sess = session.lock().await;
            for thread_id in &stale_thread_ids {
                sess.threads.remove(thread_id);
            }
        }

        // Clean up thread mappings
        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.retain(|_, &mut v| !stale_thread_ids.contains(&v));
        }

        // Clean up undo managers
        {
            let mut undo_managers = self.undo_managers.write().await;
            for thread_id in &stale_thread_ids {
                undo_managers.remove(thread_id);
            }
        }

        tracing::info!(
            "Pruned {} stale thread(s) (idle > {}s)",
            stale_thread_ids.len(),
            max_idle.as_secs()
        );

        stale_thread_ids.len()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_session_single_user() {
        let manager = SessionManager::new();

        let session1 = manager.get_session().await;
        let session2 = manager.get_session().await;

        // Single user: always same session
        assert!(Arc::ptr_eq(&session1, &session2));
    }

    #[tokio::test]
    async fn test_resolve_thread() {
        let manager = SessionManager::new();

        // Same external_thread_id (None) returns the same thread
        let (session1, thread1) = manager.resolve_thread(None).await;
        let (session2, thread2) = manager.resolve_thread(None).await;

        assert!(Arc::ptr_eq(&session1, &session2));
        assert_eq!(thread1, thread2);
    }

    // === Single-user SessionManager tests ===

    #[tokio::test]
    async fn test_get_session_returns_same_session() {
        let manager = SessionManager::new();

        let session1 = manager.get_session().await;
        let session2 = manager.get_session().await;

        // Same session should be returned
        assert!(Arc::ptr_eq(&session1, &session2));
    }

    #[tokio::test]
    async fn test_undo_manager() {
        let manager = SessionManager::new();
        let (_, thread_id) = manager.resolve_thread(None).await;

        let undo1 = manager.get_undo_manager(thread_id).await;
        let undo2 = manager.get_undo_manager(thread_id).await;

        assert!(Arc::ptr_eq(&undo1, &undo2));
    }

    #[tokio::test]
    async fn test_resolve_thread_with_explicit_external_id() {
        let manager = SessionManager::new();

        // Two calls with the same explicit external thread ID should resolve
        // to the same internal thread.
        let (_, t1) = manager.resolve_thread(Some("ext-abc")).await;
        let (_, t2) = manager.resolve_thread(Some("ext-abc")).await;
        assert_eq!(t1, t2);

        // A different external ID gets a new thread.
        let (_, t3) = manager.resolve_thread(Some("ext-xyz")).await;
        assert_ne!(t1, t3);
    }

    #[tokio::test]
    async fn test_resolve_thread_none_vs_some_external_id() {
        let manager = SessionManager::new();

        // None external_thread_id is a distinct key from Some("ext-1").
        let (_, t_none) = manager.resolve_thread(None).await;
        let (_, t_some) = manager.resolve_thread(Some("ext-1")).await;
        assert_ne!(t_none, t_some);
    }

    #[tokio::test]
    async fn test_resolve_thread_different_external_ids() {
        let manager = SessionManager::new();

        let (_, t1) = manager.resolve_thread(Some("thread-x")).await;
        let (_, t2) = manager.resolve_thread(Some("thread-y")).await;

        // Different external IDs = different threads
        assert_ne!(t1, t2);
    }

    #[tokio::test]
    async fn test_multiple_threads_different_external_ids() {
        let manager = SessionManager::new();

        let (_, t1) = manager.resolve_thread(Some("thread-a")).await;
        let (_, t2) = manager.resolve_thread(Some("thread-b")).await;
        let (session, t3) = manager.resolve_thread(Some("thread-c")).await;

        // All three should be distinct
        assert_ne!(t1, t2);
        assert_ne!(t2, t3);
        assert_ne!(t1, t3);

        // All three should exist in the same session
        let sess = session.lock().await;
        assert!(sess.threads.contains_key(&t1));
        assert!(sess.threads.contains_key(&t2));
        assert!(sess.threads.contains_key(&t3));
    }

    #[tokio::test]
    async fn test_register_thread_preserves_uuid() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let known_uuid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("default")));
        let session_id = {
            let sess = session.lock().await;
            sess.id
        };

        // Create thread with known UUID
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(known_uuid, session_id);
            sess.threads.insert(known_uuid, thread);
        }

        // Register it
        manager.register_thread(known_uuid, Arc::clone(&session)).await;

        // resolve_thread should find it
        let (_, resolved) = manager
            .resolve_thread(Some(&known_uuid.to_string()))
            .await;
        assert_eq!(resolved, known_uuid);
    }

    #[tokio::test]
    async fn test_register_thread_idempotent() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("default")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        // Register twice
        manager.register_thread(tid, Arc::clone(&session)).await;
        manager.register_thread(tid, Arc::clone(&session)).await;

        // Should still resolve to the same thread
        let (_, resolved) = manager
            .resolve_thread(Some(&tid.to_string()))
            .await;
        assert_eq!(resolved, tid);
    }

    #[tokio::test]
    async fn test_register_then_resolve_finds_it() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("default")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        // Register the thread
        manager.register_thread(tid, Arc::clone(&session)).await;

        // Resolve with same external_id should find it
        let (_, resolved) = manager
            .resolve_thread(Some(&tid.to_string()))
            .await;
        assert_eq!(resolved, tid);
    }

    #[tokio::test]
    async fn concurrent_get_session_returns_same_session() {
        let manager = Arc::new(SessionManager::new());

        let handles: Vec<_> = (0..30)
            .map(|_| {
                let mgr = Arc::clone(&manager);
                tokio::spawn(async move { mgr.get_session().await })
            })
            .collect();

        let mut sessions = Vec::new();
        for handle in handles {
            sessions.push(handle.await.expect("task should not panic"));
        }

        // All 30 must return the *same* Arc
        for s in &sessions {
            assert!(Arc::ptr_eq(&sessions[0], s));
        }
    }

    #[tokio::test]
    async fn concurrent_resolve_thread_different_external_ids() {
        let manager = Arc::new(SessionManager::new());
        let external_ids = ["thread-a", "thread-b", "thread-c", "thread-d", "thread-e"];

        let handles: Vec<_> = external_ids
            .iter()
            .map(|ext_id| {
                let mgr = Arc::clone(&manager);
                let external_id = ext_id.to_string();
                tokio::spawn(async move {
                    let (session, tid) = mgr.resolve_thread(Some(&external_id)).await;
                    (external_id, session, tid)
                })
            })
            .collect();

        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.expect("task should not panic"));
        }

        // All 5 threads must be unique
        let tids: std::collections::HashSet<_> = results.iter().map(|(_, _, t)| *t).collect();
        assert_eq!(tids.len(), 5);

        // All threads should live in the same session
        let sess = results[0].1.lock().await;
        assert_eq!(sess.threads.len(), 5);
    }

    #[tokio::test]
    async fn concurrent_get_undo_manager_same_thread_returns_same_arc() {
        let manager = Arc::new(SessionManager::new());
        let (_, tid) = manager.resolve_thread(None).await;

        let handles: Vec<_> = (0..20)
            .map(|_| {
                let mgr = Arc::clone(&manager);
                tokio::spawn(async move { mgr.get_undo_manager(tid).await })
            })
            .collect();

        let mut managers = Vec::new();
        for handle in handles {
            managers.push(handle.await.expect("task should not panic"));
        }

        // All 20 must point to the same UndoManager
        for m in &managers {
            assert!(Arc::ptr_eq(&managers[0], m));
        }
    }
}
