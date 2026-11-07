//! Who is calling. Several AI sessions (and scripts) can use wridge at once; each one keeps its own Figma
//! file (use_file) and its own file check (guards.rs), so one session switching files never moves another.
//!
//! The caller is carried as a task-local for the length of one request:
//! - MCP: the Mcp-Session-Id the server hands out on initialize (mcp.rs); an id the server does not know
//!   (e.g. after a restart) is simply a new session, never an error
//! - POST /api/tool: the X-Wridge-Session header (scripts, the CLI via WRIDGE_SESSION)
//! - nothing: the shared session, which behaves exactly like wridge did before sessions
//!
//! `Hold` makes a run of commands (run_batch) one turn in a file's queue (bridge.rs).

use std::sync::{Arc, Mutex};

pub type SessionId = Arc<str>;

/// The session of callers that say nothing about themselves.
pub const SHARED: &str = "shared";

/// The wridge window and its pages: what the person picked to look at. Never counted as an AI.
pub const APP: &str = "app";

/// The header scripts send (and the CLI, from WRIDGE_SESSION).
pub const HEADER: &str = "x-wridge-session";

tokio::task_local! {
    static SESSION: SessionId;
    static HOLD: Arc<Hold>;
}

/// The calling session, or the shared one outside a request.
pub fn current() -> SessionId {
    SESSION.try_with(|s| s.clone()).unwrap_or_else(|_| Arc::from(SHARED))
}

/// A usable session id from a header value: trimmed, 1–128 visible ASCII characters.
pub fn parse(value: Option<&str>) -> Option<SessionId> {
    let v = value?.trim();
    if v.is_empty() || v.len() > 128 || !v.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return None;
    }
    Some(Arc::from(v))
}

/// Runs `fut` as `session`.
pub async fn scope<F: std::future::Future>(session: SessionId, fut: F) -> F::Output {
    SESSION.scope(session, fut).await
}

/// A file's queue slot kept across several commands: (socket, guard). Dropping it frees the file.
#[derive(Default)]
pub struct Hold {
    pub slot: Mutex<Option<(u64, tokio::sync::OwnedMutexGuard<()>)>>,
}

/// Runs `fut` with every command it sends to one file taking a single turn in that file's queue.
pub async fn hold<F: std::future::Future>(fut: F) -> F::Output {
    if HOLD.try_with(|_| ()).is_ok() {
        return fut.await; // already inside a held run
    }
    HOLD.scope(Arc::new(Hold::default()), fut).await
}

/// The held run this task is in, if any.
pub fn current_hold() -> Option<Arc<Hold>> {
    HOLD.try_with(|h| h.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_plain_ids_only() {
        assert_eq!(parse(Some(" abc-1 ")).as_deref(), Some("abc-1"));
        assert!(parse(Some("")).is_none());
        assert!(parse(Some("   ")).is_none());
        assert!(parse(Some("a b")).is_none());
        assert!(parse(Some("ไทย")).is_none());
        assert!(parse(Some(&"x".repeat(129))).is_none());
        assert!(parse(None).is_none());
    }

    #[tokio::test]
    async fn current_is_shared_outside_a_scope() {
        assert_eq!(&*current(), SHARED);
        let inside = scope(Arc::from("s1"), async { current() }).await;
        assert_eq!(&*inside, "s1");
    }
}
