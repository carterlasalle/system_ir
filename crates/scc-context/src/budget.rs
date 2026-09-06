//! Token-budget allocators for context packs.
//!
//! Production task packs still drop lowest-priority sections
//! ([`crate::packs`] adaptive path). This module is the inspectable
//! fixed-percent + rollover alternative learned from Ripwire's packer:
//! unused quota moves forward; over-quota buckets are truncated and
//! disclosed. Do not switch the production default without ablation.

/// Atlas / Surface / Structural-exact / Call-flow / Verify / Uncertainty.
pub const TASK_CONTEXT_QUOTA_PERCENTS: [u8; 6] = [20, 25, 30, 10, 10, 5];

/// Bucket names aligned with SCC's four context levels plus verify/uncertainty.
pub const TASK_CONTEXT_QUOTA_NAMES: [&str; 6] = [
    "atlas",
    "surface",
    "source",
    "flow",
    "verify",
    "uncertainty",
];

/// One filled bucket after rollover.
#[derive(Debug, Clone, PartialEq, Eq)]
// trace:exempt reason=internal-detail
pub struct FilledBucket {
    pub name: &'static str,
    pub percent: u8,
    /// Quota after receiving unused tokens from previous buckets.
    pub quota: usize,
    /// Tokens actually granted (min(used, quota)).
    pub granted: usize,
    /// How many requested tokens did not fit.
    pub truncated: usize,
    /// Unused quota rolled to the next bucket (0 on the last).
    pub rolled_forward: usize,
}

/// Split `total` into percents, then fill each bucket from `used`, rolling
/// unused quota forward. `used.len()` must equal the percent table.
/// Last bucket absorbs leftover from integer division of percents.
// trace:v1 id=impl.scc.context.budget-rollover work=WORK-ripwire-lessons-phase3 satisfies=REQ-budget-rollover
pub fn fill_with_rollover(total: usize, percents: &[u8], names: &[&'static str], used: &[usize]) -> Vec<FilledBucket> {
    assert_eq!(percents.len(), used.len());
    assert_eq!(percents.len(), names.len());
    let sum: u16 = percents.iter().map(|p| *p as u16).sum();
    assert_eq!(sum, 100, "quota percents must sum to 100");

    let mut leftover = 0usize;
    let mut granted_so_far = 0usize;
    let mut out = Vec::with_capacity(percents.len());
    for (i, (p, u)) in percents.iter().zip(used.iter()).enumerate() {
        let base = if i + 1 == percents.len() {
            total.saturating_sub(granted_so_far + leftover)
                .saturating_add(leftover)
        } else {
            total * (*p as usize) / 100 + leftover
        };
        let take = (*u).min(base);
        let truncated = u.saturating_sub(base);
        let rolled = base.saturating_sub(*u);
        granted_so_far += take;
        leftover = rolled;
        out.push(FilledBucket {
            name: names[i],
            percent: *p,
            quota: base,
            granted: take,
            truncated,
            rolled_forward: if i + 1 == percents.len() { 0 } else { rolled },
        });
    }
    out
}

/// Convenience: SCC task-context split.
// trace:exempt reason=internal-detail
pub fn fill_task_context_quotas(total: usize, used: &[usize; 6]) -> Vec<FilledBucket> {
    fill_with_rollover(
        total,
        &TASK_CONTEXT_QUOTA_PERCENTS,
        &TASK_CONTEXT_QUOTA_NAMES,
        used,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // trace:v1 id=test.scc.context.budget-percents verifies=REQ-budget-rollover exercises=impl.scc.context.budget-rollover
    fn percents_sum_to_one_hundred() {
        let sum: u16 = TASK_CONTEXT_QUOTA_PERCENTS.iter().map(|p| *p as u16).sum();
        assert_eq!(sum, 100);
        assert_eq!(TASK_CONTEXT_QUOTA_NAMES.len(), TASK_CONTEXT_QUOTA_PERCENTS.len());
    }

    #[test]
    // trace:v1 id=test.scc.context.budget-rollover verifies=REQ-budget-rollover exercises=impl.scc.context.budget-rollover
    fn unused_quota_rolls_forward_and_truncation_is_disclosed() {
        let filled = fill_with_rollover(
            100,
            &[40, 30, 30],
            &["a", "b", "c"],
            &[10, 50, 10],
        );
        assert_eq!(filled[0].granted, 10);
        assert_eq!(filled[0].truncated, 0);
        assert_eq!(filled[0].rolled_forward, 30);
        assert_eq!(filled[1].quota, 60, "30% + 30 rolled");
        assert_eq!(filled[1].granted, 50);
        assert_eq!(filled[1].truncated, 0);
        assert_eq!(filled[1].rolled_forward, 10);
        assert_eq!(filled[2].granted, 10);
        assert_eq!(filled[2].truncated, 0);
        let over = fill_with_rollover(100, &[50, 50], &["x", "y"], &[80, 80]);
        assert_eq!(over[0].granted, 50);
        assert_eq!(over[0].truncated, 30);
        assert_eq!(over[1].granted, 50);
        assert_eq!(over[1].truncated, 30);
        let task = fill_task_context_quotas(1000, &[0, 0, 0, 0, 0, 0]);
        assert_eq!(task.iter().map(|b| b.granted).sum::<usize>(), 0);
        assert!(task.iter().any(|b| b.name == "verify"));
    }
}
