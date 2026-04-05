//! Session manager for multi-user, multi-session conversation handling.
//!
//! Each user can have multiple sessions indexed by session_id.
//! 
//! For database persistence, implement the SessionStore sub-trait on Database.

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
    owner_id: String,
    channel: String,
    external_thread_id: Option<String>,
}

/// Manages sessions, threads, and undo state for all users.
/// 
/// Sessions are indexed by owner_id, allowing each user to have multiple
/// concurrent sessions.
pub struct SessionManager {
    /// All sessions, keyed by owner_id -> session_id -> Session.
    sessions: RwLock<HashMap<String, HashMap<Uuid, Arc<Mutex<Session>>>>>,
    /// Maps ThreadKey -> (owner_id, thread_id) for thread resolution.
    thread_map: RwLock<HashMap<ThreadKey, (String, Uuid)>>,
    undo_managers: RwLock<HashMap<Uuid, Arc<Mutex<UndoManager>>>>,
    hooks: Option<Arc<HookRegistry>>,
}

impl SessionManager {
    /// Create a new session manager.
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
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

    /// Get or create the first session for the given owner.
    /// Creates a new session if none exists.
    pub async fn get_or_create_session(&self, owner_id: &str) -> Arc<Mutex<Session>> {
        // Fast path: session exists
        {
            let sessions = self.sessions.read().await;
            if let Some(user_sessions) = sessions.get(owner_id) {
                if let Some(session) = user_sessions.values().next() {
                    return Arc::clone(session);
                }
            }
        }

        // Slow path: create new session
        let mut sessions = self.sessions.write().await;
        // Double-check after acquiring write lock
        if let Some(user_sessions) = sessions.get(owner_id) {
            if let Some(session) = user_sessions.values().next() {
                return Arc::clone(session);
            }
        }

        let new_session = Session::new(owner_id);
        let session_id = new_session.id;
        let session = Arc::new(Mutex::new(new_session));

        sessions
            .entry(owner_id.to_string())
            .or_insert_with(HashMap::new)
            .insert(session_id, Arc::clone(&session));

        // Fire OnSessionStart hook
        self.fire_session_start_hook(owner_id, session_id).await;

        session
    }

    /// Create a brand-new session for the given owner.
    /// Does NOT replace existing sessions — creates a new one alongside them.
    pub async fn create_new_session(&self, owner_id: &str) -> Arc<Mutex<Session>> {
        let new_session = Session::new(owner_id);
        let session_id = new_session.id;
        let session = Arc::new(Mutex::new(new_session));

        {
            let mut sessions = self.sessions.write().await;
            sessions
                .entry(owner_id.to_string())
                .or_insert_with(HashMap::new)
                .insert(session_id, Arc::clone(&session));
        }

        // Fire OnSessionStart hook
        self.fire_session_start_hook(owner_id, session_id).await;

        session
    }

    /// Fire the OnSessionStart hook asynchronously.
    async fn fire_session_start_hook(&self, owner_id: &str, session_id: Uuid) {
        if let Some(ref hooks) = self.hooks {
            let hooks = hooks.clone();
            let owner_id = owner_id.to_string();
            let session_id_str = session_id.to_string();
            tokio::spawn(async move {
                use crate::hooks::HookEvent;
                let event = HookEvent::SessionStart {
                    user_id: owner_id.clone(),
                    session_id: session_id_str,
                };
                if let Err(e) = hooks.run(&event).await {
                    tracing::warn!("OnSessionStart hook error: {}", e);
                }
            });
        }
    }

    /// Resolve an external thread ID to an internal thread.
    ///
    /// Returns the session and thread ID. Creates both if they don't exist.
    pub async fn resolve_thread(
        &self,
        owner_id: &str,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        self.resolve_thread_with_parsed_uuid(owner_id, channel, external_thread_id, None)
            .await
    }

    /// Like [`resolve_thread`], but accepts a pre-parsed UUID.
    pub async fn resolve_thread_with_parsed_uuid(
        &self,
        owner_id: &str,
        channel: &str,
        external_thread_id: Option<&str>,
        parsed_uuid: Option<Uuid>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        let session = self.get_or_create_session(owner_id).await;
        let _session_id = session.lock().await.id;

        let key = ThreadKey {
            owner_id: owner_id.to_string(),
            channel: channel.to_string(),
            external_thread_id: external_thread_id.map(String::from),
        };

        // Use pre-parsed UUID if available, otherwise parse from string.
        let ext_uuid = parsed_uuid
            .or_else(|| external_thread_id.and_then(|ext_tid| Uuid::parse_str(ext_tid).ok()));

        // Single read lock for both the key lookup and UUID adoption check
        let adoptable = {
            let thread_map = self.thread_map.read().await;

            // Fast path: exact key match
            if let Some(&(ref oid, thread_id)) = thread_map.get(&key) {
                if oid == owner_id {
                    let sess = session.lock().await;
                    if sess.threads.contains_key(&thread_id) {
                        return (Arc::clone(&session), thread_id);
                    }
                }
            }

            // UUID adoption check (still under the same read lock).
            if external_thread_id.is_some() {
                ext_uuid.filter(|&uuid| !thread_map.values().any(|(_, v)| *v == uuid))
            } else {
                None
            }
        }; // Single read lock dropped here

        // If we found an adoptable UUID, verify it exists in session and acquire write lock
        if let Some(ext_uuid) = adoptable {
            let sess = session.lock().await;
            if sess.threads.contains_key(&ext_uuid) {
                drop(sess);

                let mut thread_map = self.thread_map.write().await;
                // Re-check after acquiring write lock
                if !thread_map.values().any(|(_, v)| *v == ext_uuid) {
                    thread_map.insert(key, (owner_id.to_string(), ext_uuid));
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
            thread_map.insert(key, (owner_id.to_string(), thread_id));
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
        owner_id: &str,
        channel: &str,
        external_thread_id: &str,
        thread_id: Uuid,
    ) -> Arc<Mutex<Session>> {
        let session = self.get_or_create_session(owner_id).await;
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
                    owner_id: owner_id.to_string(),
                    channel: channel.to_string(),
                    external_thread_id: Some(external_thread_id.to_string()),
                },
                (owner_id.to_string(), thread_id),
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
        owner_id: &str,
        channel: &str,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        let key = ThreadKey {
            owner_id: owner_id.to_string(),
            channel: channel.to_string(),
            external_thread_id: Some(thread_id.to_string()),
        };

        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.insert(key, (owner_id.to_string(), thread_id));
        }

        {
            let mut undo_managers = self.undo_managers.write().await;
            undo_managers
                .entry(thread_id)
                .or_insert_with(|| Arc::new(Mutex::new(UndoManager::new())));
        }

        // Ensure the session is registered
        {
            let mut sessions = self.sessions.write().await;
            let session_id = session.lock().await.id;
            sessions
                .entry(owner_id.to_string())
                .or_insert_with(HashMap::new)
                .insert(session_id, session);
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

    /// List all sessions for a given owner.
    pub async fn list_sessions(&self, owner_id: &str) -> Vec<(Uuid, Arc<Mutex<Session>>)> {
        let sessions = self.sessions.read().await;
        match sessions.get(owner_id) {
            Some(user_sessions) => {
                user_sessions
                    .iter()
                    .map(|(id, s)| (*id, Arc::clone(s)))
                    .collect()
            }
            None => vec![],
        }
    }

    /// Get a session by its UUID for the given owner.
    pub async fn get_session_by_id(
        &self,
        owner_id: &str,
        session_id: Uuid,
    ) -> Option<Arc<Mutex<Session>>> {
        let sessions = self.sessions.read().await;
        sessions
            .get(owner_id)
            .and_then(|user_sessions| user_sessions.get(&session_id))
            .map(Arc::clone)
    }

    /// Delete a session by its UUID for the given owner.
    /// Returns true if the session was deleted.
    pub async fn delete_session_by_id(&self, owner_id: &str, session_id: Uuid) -> bool {
        // Collect thread IDs to clean up
        let thread_ids: Option<Vec<Uuid>> = {
            let mut sessions = self.sessions.write().await;
            match sessions.get_mut(owner_id) {
                Some(user_sessions) => {
                    if let Some(session) = user_sessions.remove(&session_id) {
                        Some(
                            session
                                .try_lock()
                                .map(|s| s.threads.keys().copied().collect::<Vec<_>>())
                                .unwrap_or_default(),
                        )
                    } else {
                        None
                    }
                }
                None => None,
            }
        };

        if let Some(ids) = thread_ids {
            // Remove from thread_map
            {
                let mut thread_map = self.thread_map.write().await;
                thread_map.retain(|_, (_, tid)| !ids.contains(tid));
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
        let cutoff =
            chrono::Utc::now() - chrono::TimeDelta::seconds(max_idle.as_secs() as i64);
        let mut pruned = 0;

        let mut sessions = self.sessions.write().await;
        for (_owner_id, user_sessions) in sessions.iter_mut() {
            let stale_ids: Vec<Uuid> = user_sessions
                .iter()
                .filter(|(_, session)| {
                    session
                        .try_lock()
                        .map(|s| s.last_active_at < cutoff)
                        .unwrap_or(false)
                })
                .map(|(id, _)| *id)
                .collect();

            for id in stale_ids {
                if user_sessions.remove(&id).is_some() {
                    pruned += 1;
                }
            }
        }
        pruned
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
    async fn test_different_users_get_different_sessions() {
        let manager = SessionManager::new();

        let session1 = manager.get_or_create_session("user-1").await;
        let session2 = manager.get_or_create_session("user-2").await;

        // Different sessions for different users
        assert!(!Arc::ptr_eq(&session1, &session2));
        let id1 = session1.lock().await.id;
        let id2 = session2.lock().await.id;
        assert_ne!(id1, id2);
    }

    #[tokio::test]
    async fn test_same_user_gets_same_session() {
        let manager = SessionManager::new();

        let session1 = manager.get_or_create_session("user-1").await;
        let session2 = manager.get_or_create_session("user-1").await;

        // Same session for same user
        assert!(Arc::ptr_eq(&session1, &session2));
    }

    #[tokio::test]
    async fn test_create_new_session_creates_new_not_replace() {
        let manager = SessionManager::new();

        let s1 = manager.create_new_session("user-1").await;
        let id1 = s1.lock().await.id;

        let s2 = manager.create_new_session("user-1").await;
        let id2 = s2.lock().await.id;

        // Both exist, different IDs
        assert_ne!(id1, id2);

        // Both are in the list
        let sessions = manager.list_sessions("user-1").await;
        assert_eq!(sessions.len(), 2);
    }

    #[tokio::test]
    async fn test_list_sessions_per_owner() {
        let manager = SessionManager::new();

        // Empty initially
        assert!(manager.list_sessions("user-1").await.is_empty());

        manager.create_new_session("user-1").await;
        manager.create_new_session("user-1").await;
        manager.create_new_session("user-2").await;

        assert_eq!(manager.list_sessions("user-1").await.len(), 2);
        assert_eq!(manager.list_sessions("user-2").await.len(), 1);
        assert!(manager.list_sessions("user-3").await.is_empty());
    }

    #[tokio::test]
    async fn test_delete_session() {
        let manager = SessionManager::new();

        let session = manager.create_new_session("user-1").await;
        let id = session.lock().await.id;

        assert!(manager.delete_session_by_id("user-1", id).await);
        assert!(manager.get_session_by_id("user-1", id).await.is_none());
        assert!(manager.list_sessions("user-1").await.is_empty());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_session() {
        let manager = SessionManager::new();
        assert!(!manager.delete_session_by_id("user-1", Uuid::new_v4()).await);
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
    async fn test_register_thread() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let thread_id = Uuid::new_v4();

        // Create a session with a hydrated thread
        let session = Arc::new(Mutex::new(Session::new("user-1")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(thread_id, sess.id);
            sess.threads.insert(thread_id, thread);
            sess.active_thread = Some(thread_id);
        }

        // Register the thread
        manager
            .register_thread("user-1", "gateway", thread_id, Arc::clone(&session))
            .await;

        // resolve_thread should find it
        let (_, resolved) = manager
            .resolve_thread("user-1", "gateway", Some(&thread_id.to_string()))
            .await;
        assert_eq!(resolved, thread_id);
    }

    #[tokio::test]
    async fn test_create_bound_thread() {
        let manager = SessionManager::new();
        let thread_id = Uuid::new_v4();

        let session = manager
            .create_bound_thread("user-1", "cli", "ext-123", thread_id)
            .await;

        let sess = session.lock().await;
        assert!(sess.threads.contains_key(&thread_id));
        assert_eq!(sess.active_thread, Some(thread_id));
    }
}
