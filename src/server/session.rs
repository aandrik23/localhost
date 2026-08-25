//! In-memory session store.
//!
//! Sessions are kept in a HashMap owned by the single server thread,
//! so no synchronization primitives are needed. Expiration is swept
//! from the main event loop; there is no background cleanup thread.

use std::collections::HashMap;
use std::time::{
    Duration,
    Instant,
};

/// How long a session may go unused before it is considered expired.
pub const SESSION_TTL: Duration = Duration::from_secs(30 * 60);

/// Name of the cookie used to carry the session id.
pub const SESSION_COOKIE_NAME: &str = "session_id";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parses a session id received from a client-controlled Cookie
    /// header. Only accepts the exact shape generate() produces (32
    /// lowercase hex characters) so a malformed or hostile cookie
    /// value can never be mistaken for a real session id.
    pub fn parse(value: &str) -> Option<Self> {
        if value.len() != 32
            || !value.bytes().all(|byte| {
                byte.is_ascii_hexdigit()
                    && !byte.is_ascii_uppercase()
            })
        {
            return None;
        }

        Some(SessionId(value.to_string()))
    }

    /// Generates a new session id from 16 bytes of OS randomness,
    /// hex-encoded. getrandom(2) is the only unsafe call in this
    /// module; its safety invariant is that `bytes` is a valid,
    /// appropriately-sized buffer for the duration of the call,
    /// which a fixed-size stack array guarantees.
    fn generate() -> Self {
        let mut bytes = [0u8; 16];

        let result = unsafe {
            libc::getrandom(
                bytes.as_mut_ptr() as *mut libc::c_void,
                bytes.len(),
                0,
            )
        };

        if result != bytes.len() as isize {
            /*
             * getrandom() should not fail or short-read for a
             * 16-byte request under normal operation. Falling back
             * to a time-derived value keeps the server available
             * (never crash) instead of panicking; this path is not
             * expected to be exercised outside of a broken sandbox.
             */
            let fallback =
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();

            bytes = fallback.to_le_bytes()[..16]
                .try_into()
                .unwrap_or(bytes);
        }

        let hex: String = bytes
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect();

        SessionId(hex)
    }
}

pub struct Session {
    pub id: SessionId,

    pub created_at: Instant,
    pub last_accessed: Instant,

    pub data: HashMap<String, String>,
}

impl Session {
    fn new() -> Self {
        let now = Instant::now();

        Self {
            id: SessionId::generate(),
            created_at: now,
            last_accessed: now,
            data: HashMap::new(),
        }
    }
}

pub struct SessionStore {
    sessions: HashMap<SessionId, Session>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Looks up an existing, non-expired session by id and marks it
    /// as accessed. Returns None if the id is unknown or expired -
    /// callers should treat that the same as "no session cookie" and
    /// create a fresh one.
    pub fn touch(
        &mut self,
        id: &SessionId,
    ) -> Option<&mut Session> {
        let session = self.sessions.get_mut(id)?;

        if session.last_accessed.elapsed() > SESSION_TTL {
            self.sessions.remove(id);

            return None;
        }

        session.last_accessed = Instant::now();

        self.sessions.get_mut(id)
    }

    /// Creates a new session and inserts it into the store, returning
    /// a reference to it.
    pub fn create(&mut self) -> &mut Session {
        let session = Session::new();

        let id = session.id.clone();

        self.sessions.insert(id.clone(), session);

        self.sessions
            .get_mut(&id)
            .expect("session was just inserted")
    }

    /// Removes every session whose last access is older than
    /// SESSION_TTL. Intended to be called periodically from the main
    /// event loop, never from a background thread.
    pub fn sweep_expired(&mut self) {
        self.sessions.retain(|_, session| {
            session.last_accessed.elapsed() <= SESSION_TTL
        });
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_touch_round_trip() {
        let mut store = SessionStore::new();

        let id = store.create().id.clone();

        assert!(store.touch(&id).is_some());
    }

    #[test]
    fn touch_unknown_id_returns_none() {
        let mut store = SessionStore::new();

        let unknown = SessionId::generate();

        assert!(store.touch(&unknown).is_none());
    }

    #[test]
    fn generated_ids_are_unique() {
        let a = SessionId::generate();
        let b = SessionId::generate();

        assert_ne!(a, b);
    }

    #[test]
    fn generated_id_is_32_hex_chars() {
        let id = SessionId::generate();

        assert_eq!(id.as_str().len(), 32);

        assert!(
            id.as_str()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
    }

    #[test]
    fn sweep_expired_removes_only_stale_sessions() {
        let mut store = SessionStore::new();

        let fresh_id = store.create().id.clone();

        // Simulate an already-expired session by backdating
        // last_accessed past the TTL.
        let stale_id = store.create().id.clone();

        if let Some(session) = store.sessions.get_mut(&stale_id) {
            session.last_accessed =
                Instant::now() - SESSION_TTL - Duration::from_secs(1);
        }

        store.sweep_expired();

        assert!(store.touch(&fresh_id).is_some());
        assert_eq!(store.len(), 1);
    }
}
