//! The session manager — owns the set of live sessions by name and id.
//! Ported from TUIOS `internal/session/manager.go`.

use std::collections::HashMap;
use std::sync::Mutex;

use super::model::{validate_session_name, Session, SessionConfig, SessionInfo};

/// An error from a manager operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerError {
    AlreadyExists(String),
    NotFound(String),
    InvalidName(String),
}

impl std::fmt::Display for ManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManagerError::AlreadyExists(n) => write!(f, "session '{n}' already exists"),
            ManagerError::NotFound(n) => write!(f, "session '{n}' not found"),
            ManagerError::InvalidName(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ManagerError {}

/// The live-session registry.
#[derive(Default)]
pub struct Manager {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Session>,
    by_id: HashMap<String, Session>,
}

impl Manager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new session, validating its name and uniqueness.
    pub fn create(&self, name: &str, _cfg: &SessionConfig) -> Result<Session, ManagerError> {
        validate_session_name(name).map_err(ManagerError::InvalidName)?;
        let mut inner = self.inner.lock().unwrap();
        if inner.sessions.contains_key(name) {
            return Err(ManagerError::AlreadyExists(name.to_string()));
        }
        let session = Session::new(name);
        inner.sessions.insert(name.to_string(), session.clone());
        inner.by_id.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    /// Register a session restored from saved state.
    pub fn restore(&self, name: &str) -> Result<Session, ManagerError> {
        validate_session_name(name).map_err(ManagerError::InvalidName)?;
        let mut inner = self.inner.lock().unwrap();
        if inner.sessions.contains_key(name) {
            return Err(ManagerError::AlreadyExists(name.to_string()));
        }
        let session = Session::restored(name);
        inner.sessions.insert(name.to_string(), session.clone());
        inner.by_id.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    pub fn get(&self, name: &str) -> Option<Session> {
        self.inner.lock().unwrap().sessions.get(name).cloned()
    }

    pub fn get_by_id(&self, id: &str) -> Option<Session> {
        self.inner.lock().unwrap().by_id.get(id).cloned()
    }

    /// Return an existing session or create a new one. The bool is true when
    /// a new session was created.
    pub fn get_or_create(
        &self,
        name: &str,
        cfg: &SessionConfig,
    ) -> Result<(Session, bool), ManagerError> {
        if let Some(s) = self.get(name) {
            return Ok((s, false));
        }
        let s = self.create(name, cfg)?;
        Ok((s, true))
    }

    /// Remove a session from the registry.
    pub fn delete(&self, name: &str) -> Result<Session, ManagerError> {
        let mut inner = self.inner.lock().unwrap();
        let session = inner
            .sessions
            .remove(name)
            .ok_or_else(|| ManagerError::NotFound(name.to_string()))?;
        inner.by_id.remove(&session.id);
        Ok(session)
    }

    /// All sessions, ordered by creation time then name.
    pub fn list(&self) -> Vec<Session> {
        let inner = self.inner.lock().unwrap();
        let mut sessions: Vec<Session> = inner.sessions.values().cloned().collect();
        sessions.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.name.cmp(&b.name))
        });
        sessions
    }

    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().sessions.len()
    }

    pub fn has_sessions(&self) -> bool {
        self.count() > 0
    }

    /// The lowest available `session-N` name.
    pub fn generate_name(&self) -> String {
        let inner = self.inner.lock().unwrap();
        for i in 0.. {
            let name = format!("session-{i}");
            if !inner.sessions.contains_key(&name) {
                return name;
            }
        }
        unreachable!()
    }

    /// The default session: the first one, creating a new named session if
    /// none exist.
    pub fn default_session(&self, cfg: &SessionConfig) -> Result<Session, ManagerError> {
        if let Some(s) = self.list().into_iter().next() {
            return Ok(s);
        }
        let name = self.generate_name();
        self.create(&name, cfg)
    }
}

/// A snapshot of the manager state for tests and the `list` command.
pub fn info_for(session: &Session, windows: usize, attached: bool) -> SessionInfo {
    SessionInfo {
        id: session.id.clone(),
        name: session.name.clone(),
        created_at: session.created_at,
        attached,
        windows,
        restored: session.restored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SessionConfig {
        SessionConfig::default()
    }

    #[test]
    fn create_and_lookup() {
        let m = Manager::new();
        let s = m.create("dev", &cfg()).unwrap();
        assert_eq!(m.get("dev").unwrap().id, s.id);
        assert_eq!(m.get_by_id(&s.id).unwrap().name, "dev");
        assert_eq!(m.count(), 1);
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let m = Manager::new();
        m.create("dev", &cfg()).unwrap();
        assert_eq!(
            m.create("dev", &cfg()).unwrap_err(),
            ManagerError::AlreadyExists("dev".to_string())
        );
    }

    #[test]
    fn get_or_create_returns_existing() {
        let m = Manager::new();
        let first = m.create("dev", &cfg()).unwrap();
        let (second, created) = m.get_or_create("dev", &cfg()).unwrap();
        assert!(!created);
        assert_eq!(first.id, second.id);
        let (_, created) = m.get_or_create("other", &cfg()).unwrap();
        assert!(created);
    }

    #[test]
    fn delete_and_generate_name() {
        let m = Manager::new();
        m.create("session-0", &cfg()).unwrap();
        m.create("session-2", &cfg()).unwrap();
        assert_eq!(m.generate_name(), "session-1");
        m.delete("session-0").unwrap();
        assert_eq!(m.count(), 1);
        assert!(m.delete("nope").is_err());
    }

    #[test]
    fn list_is_ordered() {
        let m = Manager::new();
        m.create("b", &cfg()).unwrap();
        m.create("a", &cfg()).unwrap();
        let names: Vec<String> = m.list().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn new_manager_is_empty() {
        let m = Manager::new();
        assert_eq!(m.count(), 0);
        assert!(m.list().is_empty());
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let m = Manager::new();
        assert!(m.get("nope").is_none());
    }

    #[test]
    fn get_by_id_nonexistent_returns_none() {
        let m = Manager::new();
        assert!(m.get_by_id("nonexistent-id").is_none());
    }

    #[test]
    fn delete_decreases_count() {
        let m = Manager::new();
        m.create("a", &cfg()).unwrap();
        m.create("b", &cfg()).unwrap();
        assert_eq!(m.count(), 2);
        m.delete("a").unwrap();
        assert_eq!(m.count(), 1);
    }

    #[test]
    fn generate_name_empty() {
        let m = Manager::new();
        assert_eq!(m.generate_name(), "session-0");
    }

    #[test]
    fn generate_name_fills_gap() {
        let m = Manager::new();
        m.create("session-0", &cfg()).unwrap();
        m.create("session-2", &cfg()).unwrap();
        assert_eq!(m.generate_name(), "session-1");
    }

    #[test]
    fn create_multiple_and_lookup() {
        let m = Manager::new();
        m.create("alpha", &cfg()).unwrap();
        m.create("beta", &cfg()).unwrap();
        m.create("gamma", &cfg()).unwrap();
        assert_eq!(m.count(), 3);
        assert!(m.get("alpha").is_some());
        assert!(m.get("beta").is_some());
        assert!(m.get("gamma").is_some());
    }
}
