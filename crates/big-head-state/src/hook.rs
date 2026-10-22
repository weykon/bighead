//! State hook — cloneable broadcast-based observable, following the `LogEmitter` pattern.
//!
//! [`StateHook`] is cheap to clone (wraps `broadcast::Sender`).
//! [`ActivityGuard`] provides RAII-scoped emissions.

use tokio::sync::broadcast;

use crate::AgentActivity;

/// Cloneable, cheap state emission hook.
///
/// Pattern: `AgentRuntime` owns one, clones propagate to `Agent` and
/// `RuntimeToolExecutor`. Each calls `emit()` or creates a `guard()` at
/// key lifecycle points.
///
/// Under the hood uses a `broadcast` channel — slow receivers don't
/// block senders (messages are dropped if the channel lags).
#[derive(Clone, Debug)]
pub struct StateHook {
    tx: broadcast::Sender<AgentActivity>,
}

impl StateHook {
    /// Create a new hook with the given channel capacity.
    ///
    /// Capacity determines how many unread activities can accumulate
    /// before the oldest are dropped.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Emit an activity to all subscribers.
    ///
    /// Non-blocking: if the channel is full, the activity is silently dropped.
    pub fn emit(&self, activity: AgentActivity) {
        let _ = self.tx.send(activity);
    }

    /// Create a new subscriber for this hook.
    ///
    /// Each subscriber receives all future emissions.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentActivity> {
        self.tx.subscribe()
    }

    /// Create an RAII [`ActivityGuard`] that emits `start` on creation
    /// and `end` on drop.
    ///
    /// Useful for scoped activities like reasoning waves:
    ///
    /// ```ignore
    /// let _guard = hook.guard(
    ///     AgentActivity::Reasoning { wave: 3 },
    ///     AgentActivity::Idle,
    /// );
    /// ```
    pub fn guard(&self, start: AgentActivity, end: AgentActivity) -> ActivityGuard {
        self.emit(start);
        ActivityGuard {
            hook: self.clone(),
            end: Some(end),
        }
    }
}

/// RAII guard that emits an activity on creation and another on drop.
///
/// Created via [`StateHook::guard()`].
#[must_use = "ActivityGuard emits on drop; bind to a variable to control lifetime"]
pub struct ActivityGuard {
    hook: StateHook,
    end: Option<AgentActivity>,
}

impl ActivityGuard {
    /// Prevent the end activity from being emitted on drop.
    pub fn cancel(mut self) {
        self.end = None;
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        if let Some(end) = self.end.take() {
            self.hook.emit(end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_and_receive() {
        let hook = StateHook::new(16);
        let mut rx = hook.subscribe();

        hook.emit(AgentActivity::Idle);
        let received = rx.try_recv().unwrap();
        assert!(matches!(received, AgentActivity::Idle));
    }

    #[test]
    fn test_guard_emits_start_and_end() {
        let hook = StateHook::new(16);
        let mut rx = hook.subscribe();

        {
            let _guard = hook.guard(AgentActivity::Reasoning { wave: 1 }, AgentActivity::Idle);
            // Guard emits Reasoning on creation
            let received = rx.try_recv().unwrap();
            assert!(matches!(received, AgentActivity::Reasoning { wave: 1 }));
        }
        // Guard emits Idle on drop
        let received = rx.try_recv().unwrap();
        assert!(matches!(received, AgentActivity::Idle));
    }

    #[test]
    fn test_guard_cancel() {
        let hook = StateHook::new(16);
        let mut rx = hook.subscribe();

        {
            let guard = hook.guard(AgentActivity::Reasoning { wave: 1 }, AgentActivity::Idle);
            let _ = rx.try_recv(); // consume start
            guard.cancel();
        }
        // Should not receive end
        assert!(rx.try_recv().is_err());
    }
}
