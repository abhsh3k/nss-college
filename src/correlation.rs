//! Per-request correlation ids for the audit log.
//!
//! One HTTP request can write several audit rows — an appointment request also
//! notifies the Principal, for example — and an operator reconstructing an
//! incident needs to see them as one action. A middleware stamps every request
//! with an id, [`users::audit`](crate::services::users::audit) reads it, and the
//! rows written while that request is handled carry the same value.
//!
//! The id is generated server-side and never read from the request, so a client
//! cannot forge one. It is not a secret: it is only an opaque grouping key.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

tokio::task_local! {
    static CORRELATION_ID: String;
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh id, unique within this process. It is not a cryptographic token and
/// is never used as one; it only has to be stable for one request and distinct
/// from every other request's id. The counter makes two requests in the same
/// nanosecond distinct; the timestamp makes restarts distinct.
pub fn new_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Address entropy, so two processes started in the same nanosecond (e.g.
    // tests in parallel) do not collide.
    let entropy = &COUNTER as *const _ as usize;
    format!("{:x}{:016x}{:x}", now.as_nanos(), seq, entropy)
}

/// The id of the request being handled, if the audit write is inside one.
/// `None` for background work (the scheduler) and for direct service calls.
pub fn current() -> Option<String> {
    CORRELATION_ID.try_with(|v| v.clone()).ok()
}

/// Run `future` with `id` visible to [`current`].
pub async fn scope<F: Future>(id: String, future: F) -> F::Output {
    CORRELATION_ID.scope(id, future).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_do_not_repeat() {
        let ids: std::collections::HashSet<String> = (0..1000).map(|_| new_id()).collect();
        assert_eq!(ids.len(), 1000, "every id should be distinct");
    }

    #[tokio::test]
    async fn the_id_is_visible_only_inside_its_scope() {
        assert!(current().is_none(), "no id outside a request");
        let seen = scope("abc".to_string(), async move { current() }).await;
        assert_eq!(seen.as_deref(), Some("abc"));
        assert!(current().is_none(), "the scope does not leak");
    }
}
