//! Cache eligibility is independent of source freshness and scheduling.
use serde_json::{Value, json};

#[derive(Default)]
pub(crate) struct Policy<'a> {
    pub source_pointer: Option<&'a str>,
    pub stale_threshold_ms: Option<u64>,
    pub refresh_interval_ms: u64,
    pub previous_receipt: Option<&'a str>,
}

pub(crate) fn reuse(policy: &Policy<'_>, key: &str, now: u64) -> Option<Value> {
    if policy.refresh_interval_ms == 0 {
        return None;
    }
    let receipt: Value = serde_json::from_str(policy.previous_receipt?).ok()?;
    if receipt["cache_key"].as_str()? != key || !receipt["data"].is_array() {
        return None;
    }
    let executed_at = receipt["executed_at"].as_u64()?;
    let elapsed = now.checked_sub(executed_at)?.saturating_mul(1000);
    if elapsed >= policy.refresh_interval_ms {
        return None;
    }
    let mut reused = receipt;
    reused["cache_hit"] = json!(true);
    reused["observed_at"] = json!(now);
    Some(reused)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_uses_execution_time_not_the_latest_hit() {
        let original = json!({"cache_key":"key","data":[],"executed_at":100}).to_string();
        let policy = Policy {
            refresh_interval_ms: 300000,
            previous_receipt: Some(&original),
            ..Default::default()
        };
        let hit = reuse(&policy, "key", 399).unwrap().to_string();
        let policy = Policy {
            previous_receipt: Some(&hit),
            ..policy
        };
        assert!(reuse(&policy, "key", 400).is_none());
        assert!(reuse(&policy, "changed", 399).is_none());
        assert!(reuse(&policy, "key", 99).is_none());
        assert!(
            reuse(
                &Policy {
                    refresh_interval_ms: 0,
                    ..policy
                },
                "key",
                399
            )
            .is_none()
        );
    }
}
