//! Session manager for single-user, multi-thread conversation handling.
//!
//! Manages a single session with multiple threads and undo state.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::agent::session::Session;
use crate::agent::undo::UndoManager;
use crate::hooks::HookRegistry;

/// Warn when session count exceeds this threshold.
#[allow(dead_code)]
const SESSION_COUNT_WARNING_THRESHOLD: usize = 1000;

/// Key for mapping external thread IDs to internal ones.
#[derive(Clone, Hash, Eq, PartialEq)]
struct ThreadKey {
    channel: String,
    external_thread_id: Option<String>,
}

/// Manages the single session and its threads for single-user mode.
pub struct SessionManager {
    /// The single session for this user.
    session: RwLock<Option<Arc<Mutex<Session>>>>,
    thread_map: RwLock<HashMap<ThreadKey, Uuid>>,
    undo_managers: RwLock<HashMap<Uuid, Arc<Mutex<UndoManager>>>>,
    hooks: Option<Arc<HookRegistry>>,
}

impl SessionManager {
    /// Create a new session manager.
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
    /// Ignores user_id - always returns the same session for single-user mode.
    pub async fn get_or_create_session(&self, _user_id: &str) -> Arc<Mutex<Session>> {
        // Fast path: session exists
        {
            let sessions = self.session.read().await;
            if let Some(ref session) = *sessions {
                return Arc::clone(session);
            }
        }

        // Slow path: create session
        let mut sessions = self.session.write().await;
        // Double-check after acquiring write lock
        if let Some(ref session) = *sessions {
            return Arc::clone(session);
        }

        let new_session = Session::new("default");
        let session_id = new_session.id.to_string();
        let session = Arc::new(Mutex::new(new_session));
        *sessions = Some(Arc::clone(&session));

        // Fire OnSessionStart hook (fire-and-forget)
        if let Some(ref hooks) = self.hooks {
            let hooks = hooks.clone();
            tokio::spawn(async move {
                use crate::hooks::HookEvent;
                let event = HookEvent::SessionStart {
                    user_id: "default".to_string(),
                    session_id: session_id,
                };
                if let Err(e) = hooks.run(&event).await {
                    tracing::warn!("OnSessionStart hook error: {}", e);
                }
            });
        }

        session
    }

    /// Create a brand-new session (always creates, replacing any existing session).
    pub async fn create_new_session(&self, _user_id: &str) -> Arc<Mutex<Session>> {
        // Clean up old session's threads from thread_map and undo_managers
        let old_thread_ids: Vec<Uuid> = {
            let mut sessions = self.session.write().await;
            let old_ids = sessions.take().map(|old_sess| {
                let sess = old_sess.try_lock().map(|s| {
                    s.threads.keys().copied().collect::<Vec<_>>()
                }).unwrap_or_default();
                sess
            }).unwrap_or_default();
            old_ids
        };

        // Remove old threads from thread_map and undo_managers
        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.retain(|_, thread_id| !old_thread_ids.contains(thread_id));
        }
        {
            let mut undo_managers = self.undo_managers.write().await;
            for thread_id in &old_thread_ids {
                undo_managers.remove(thread_id);
            }
        }

        let new_session = Session::new("default");
        let session_id = new_session.id.to_string();
        let session = Arc::new(Mutex::new(new_session));

        {
            let mut sessions = self.session.write().await;
            *sessions = Some(Arc::clone(&session));
        }

        // Fire OnSessionStart hook (fire-and-forget)
        if let Some(ref hooks) = self.hooks {
            let hooks = hooks.clone();
            tokio::spawn(async move {
                use crate::hooks::HookEvent;
                let event = HookEvent::SessionStart {
                    user_id: "default".to_string(),
                    session_id: session_id,
                };
                if let Err(e) = hooks.run(&event).await {
                    tracing::warn!("OnSessionStart hook error: {}", e);
                }
            });
        }

        session
    }

    /// Resolve an external thread ID to an internal thread.
    ///
    /// Returns the session and thread ID. Creates both if they don't exist.
    pub async fn resolve_thread(
        &self,
        _user_id: &str,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        self.resolve_thread_with_parsed_uuid(_user_id, channel, external_thread_id, None)
            .await
    }

    /// Like [`resolve_thread`], but accepts a pre-parsed UUID.
    pub async fn resolve_thread_with_parsed_uuid(
        &self,
        _user_id: &str,
        channel: &str,
        external_thread_id: Option<&str>,
        parsed_uuid: Option<Uuid>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        let session = self.get_or_create_session("default").await;

        let key = ThreadKey {
            channel: channel.to_string(),
            external_thread_id: external_thread_id.map(String::from),
        };

        // Use pre-parsed UUID if available, otherwise parse from string.
        let ext_uuid = parsed_uuid
            .or_else(|| external_thread_id.and_then(|ext_tid| Uuid::parse_str(ext_tid).ok()));

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

            // UUID adoption check (still under the same read lock).
            if external_thread_id.is_some() {
                ext_uuid.filter(|&uuid| !thread_map.values().any(|&v| v == uuid))
            } else {
                None
            }
        }; // Single read lock dropped here

        // If we found an adoptable UUID, verify it exists in session and acquire write lock
        if let Some(ext_uuid) = adoptable_uuid {
            let sess = session.lock().await;
            if sess.threads.contains_key(&ext_uuid) {
                drop(sess);

                let mut thread_map = self.thread_map.write().await;
                // Re-check after acquiring write lock
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
    pub async fn create_bound_thread(
        &self,
        _user_id: &str,
        channel: &str,
        external_thread_id: &str,
        thread_id: Uuid,
    ) -> Arc<Mutex<Session>> {
        let session = self.get_or_create_session("default").await;
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
                    channel: channel.to_string(),
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
        _user_id: &str,
        channel: &str,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        let key = ThreadKey {
            channel: channel.to_string(),
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

        // Ensure the session is set
        {
            let mut sessions = self.session.write().await;
            if sessions.is_none() {
                *sessions = Some(session);
            }
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
        // Double-check
        if let Some(mgr) = managers.get(&thread_id) {
            return Arc::clone(mgr);
        }

        let mgr = Arc::new(Mutex::new(UndoManager::new()));
        managers.insert(thread_id, Arc::clone(&mgr));
        mgr
    }

    /// List all sessions (always returns the single session if present).
    pub async fn list_sessions(&self) -> Vec<(String, Arc<Mutex<Session>>)> {
        let sessions = self.session.read().await;
        match &*sessions {
            Some(s) => vec![("default".to_string(), Arc::clone(s))],
            None => vec![],
        }
    }

    /// Get a session by its UUID.
    pub async fn get_session_by_id(&self, session_id: Uuid) -> Option<Arc<Mutex<Session>>> {
        let sessions = self.session.read().await;
        match &*sessions {
            Some(s) if s.try_lock().map(|l| l.id == session_id).unwrap_or(false) => {
                Some(Arc::clone(s))
            }
            _ => None,
        }
    }

    /// Delete a session by its UUID.
    /// Returns true if the session was deleted.
    pub async fn delete_session_by_id(&self, session_id: Uuid) -> bool {
        // Collect thread IDs to clean up
        let thread_ids: Option<Vec<Uuid>> = {
            let mut sessions = self.session.write().await;
            match &*sessions {
                Some(s) if s.try_lock().map(|l| l.id == session_id).unwrap_or(false) => {
                    let ids = s.try_lock().map(|sess| {
                        sess.threads.keys().copied().collect::<Vec<_>>()
                    }).unwrap_or_default();
                    *sessions = None;
                    Some(ids)
                }
                _ => None,
            }
        };

        if let Some(ids) = thread_ids {
            // Remove from thread_map
            {
                let mut thread_map = self.thread_map.write().await;
                thread_map.retain(|_, thread_id| !ids.contains(thread_id));
            }
            // Remove from undo_managers
            {
                let mut undo_managers = self.undo_managers.write().await;
                for tid in &ids {
                    undo_managers.remove(tid);
                }
            }
            true
        } else {
            false
        }
    }

    pub async fn prune_stale_sessions(&self, max_idle: std::time::Duration) -> usize {
        let cutoff = chrono::Utc::now() - chrono::TimeDelta::seconds(max_idle.as_secs() as i64);

        let mut sessions = self.session.write().await;
        let Some(ref session) = *sessions else { return 0; };

        let stale = session
            .try_lock()
            .map(|sess| sess.last_active_at < cutoff)
            .unwrap_or(false);

        if stale {
            *sessions = None;
            return 1;
        }
        0
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
    async fn test_single_session_always_returns_same() {
        let manager = SessionManager::new();

        let session1 = manager.get_or_create_session("user-1").await;
        let session2 = manager.get_or_create_session("user-2").await;

        // Same session regardless of user_id
        assert!(Arc::ptr_eq(&session1, &session2));
    }

    #[tokio::test]
    async fn test_resolve_thread() {
        let manager = SessionManager::new();

        let (session1, thread1) = manager.resolve_thread("user-1", "cli", None).await;
        let (session2, thread2) = manager.resolve_thread("user-1", "cli", None).await;

        // Same thread for same channel
        assert!(Arc::ptr_eq(&session1, &session2));
        assert_eq!(thread1, thread2);

        // Different channel gets different thread
        let (_, thread3) = manager.resolve_thread("user-1", "http", None).await;
        assert_ne!(thread1, thread3);
    }

    #[tokio::test]
    async fn test_undo_manager() {
        let manager = SessionManager::new();
        let (_, thread_id) = manager.resolve_thread("user-1", "cli", None).await;

        let undo1 = manager.get_undo_manager(thread_id).await;
        let undo2 = manager.get_undo_manager(thread_id).await;

        assert!(Arc::ptr_eq(&undo1, &undo2));
    }

    #[tokio::test]
    async fn test_delete_session_by_id() {
        let manager = SessionManager::new();

        let session = manager.create_new_session("default").await;
        let id = session.lock().await.id;

        assert!(manager.delete_session_by_id(id).await);
        assert!(manager.get_session_by_id(id).await.is_none());
    }

    #[tokio::test]
    async fn test_create_new_session_replaces_old() {
        let manager = SessionManager::new();

        let s1 = manager.create_new_session("default").await;
        let id1 = s1.lock().await.id;

        let s2 = manager.create_new_session("default").await;
        let id2 = s2.lock().await.id;

        // Different sessions
        assert_ne!(id1, id2);
    }

    #[tokio::test]
    async fn test_list_sessions_single_user() {
        let manager = SessionManager::new();

        // Empty initially
        assert!(manager.list_sessions().await.is_empty());

        manager.create_new_session("default").await;
        let sessions = manager.list_sessions().await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].0, "default");
    }

    #[tokio::test]
    async fn test_register_thread() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let thread_id = Uuid::new_v4();

        // Create a session with a hydrated thread
        let session = Arc::new(Mutex::new(Session::new("default")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(thread_id, sess.id);
            sess.threads.insert(thread_id, thread);
            sess.active_thread = Some(thread_id);
        }

        // Register the thread
        manager
            .register_thread("default", "gateway", thread_id, Arc::clone(&session))
            .await;

        // resolve_thread should find it
        let (_, resolved) = manager
            .resolve_thread("default", "gateway", Some(&thread_id.to_string()))
            .await;
        assert_eq!(resolved, thread_id);
    }
}
