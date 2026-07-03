use std::collections::HashMap;

pub const SMALL_REPO_THRESHOLD: u32 = 20;
pub const BUCKET_TYPES: [&str; 6] = [
    "file_page",
    "symbol_spotlight",
    "module_page",
    "api_contract",
    "infra_page",
    "scc_page",
];
pub const BUCKET_FLOOR: [(&str, u32); 6] = [
    ("file_page", 0),
    ("symbol_spotlight", 0),
    ("module_page", 0),
    ("api_contract", 1),
    ("infra_page", 1),
    ("scc_page", 1),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BucketAllocation {
    pub file_page: u32,
    pub symbol_spotlight: u32,
    pub module_page: u32,
    pub api_contract: u32,
    pub infra_page: u32,
    pub scc_page: u32,
}

impl BucketAllocation {
    pub fn total(&self) -> u32 {
        self.file_page
            + self.symbol_spotlight
            + self.module_page
            + self.api_contract
            + self.infra_page
            + self.scc_page
    }
}

pub fn compute_budget(n_files: u32, coverage_pct: f64) -> u32 {
    if n_files == 0 {
        return 0;
    }

    let pct = coverage_pct.clamp(0.0, 1.0);
    let raw = (n_files as f64 * pct) as u32;

    if n_files <= SMALL_REPO_THRESHOLD {
        raw.max(n_files)
    } else {
        raw
    }
}

pub fn allocate_budget(
    budget: u32,
    candidates_per_bucket: &HashMap<String, u32>,
    shares: &HashMap<String, f64>,
    n_files: u32,
) -> BucketAllocation {
    let total_available = candidates_per_bucket.values().copied().sum::<u32>();

    if total_available > 0 && budget >= total_available {
        return BucketAllocation {
            file_page: candidates_per_bucket.get("file_page").copied().unwrap_or(0),
            symbol_spotlight: candidates_per_bucket
                .get("symbol_spotlight")
                .copied()
                .unwrap_or(0),
            module_page: candidates_per_bucket
                .get("module_page")
                .copied()
                .unwrap_or(0),
            api_contract: candidates_per_bucket
                .get("api_contract")
                .copied()
                .unwrap_or(0),
            infra_page: candidates_per_bucket
                .get("infra_page")
                .copied()
                .unwrap_or(0),
            scc_page: candidates_per_bucket.get("scc_page").copied().unwrap_or(0),
        };
    }

    let small_repo_floor = if n_files > 0 && n_files <= SMALL_REPO_THRESHOLD {
        1
    } else {
        0
    };
    let mut raw = HashMap::new();

    for bucket in BUCKET_TYPES {
        let share = shares.get(bucket).copied().unwrap_or(0.0).max(0.0);
        let mut target = (budget as f64 * share).round() as u32;
        target = target.max(bucket_floor(bucket));
        let available = candidates_per_bucket.get(bucket).copied().unwrap_or(0);
        if available > 0 {
            target = target.max(small_repo_floor);
        }
        target = target.min(available);
        raw.insert(bucket.to_string(), target);
    }

    let spent = raw.values().copied().sum::<u32>();
    let spill = budget.saturating_sub(spent);
    if spill > 0 {
        let max_file = candidates_per_bucket.get("file_page").copied().unwrap_or(0);
        let raw_file = raw.get("file_page").copied().unwrap_or(0);
        raw.insert(
            "file_page".to_string(),
            max_file.min(raw_file.saturating_add(spill)),
        );
    }

    let mut over = raw.values().copied().sum::<u32>().saturating_sub(budget);
    for bucket in BUCKET_TYPES.iter().rev() {
        if over == 0 {
            break;
        }

        let value = raw.get(*bucket).copied().unwrap_or(0);
        let reduction = value.min(over);
        raw.insert((*bucket).to_string(), value - reduction);
        over -= reduction;
    }

    BucketAllocation {
        file_page: raw.get("file_page").copied().unwrap_or(0),
        symbol_spotlight: raw.get("symbol_spotlight").copied().unwrap_or(0),
        module_page: raw.get("module_page").copied().unwrap_or(0),
        api_contract: raw.get("api_contract").copied().unwrap_or(0),
        infra_page: raw.get("infra_page").copied().unwrap_or(0),
        scc_page: raw.get("scc_page").copied().unwrap_or(0),
    }
}

pub fn default_shares() -> HashMap<String, f64> {
    HashMap::from([
        ("file_page".to_string(), 0.50),
        ("symbol_spotlight".to_string(), 0.15),
        ("module_page".to_string(), 0.10),
        ("api_contract".to_string(), 0.08),
        ("infra_page".to_string(), 0.05),
        ("scc_page".to_string(), 0.04),
    ])
}

fn bucket_floor(bucket: &str) -> u32 {
    BUCKET_FLOOR
        .iter()
        .find(|(bucket_type, _)| *bucket_type == bucket)
        .map(|(_, floor)| *floor)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(entries: &[(&str, u32)]) -> HashMap<String, u32> {
        entries
            .iter()
            .map(|(bucket, count)| ((*bucket).to_string(), *count))
            .collect()
    }

    #[test]
    fn compute_budget_applies_small_repo_floor() {
        assert_eq!(compute_budget(10, 0.5), 10);
    }

    #[test]
    fn compute_budget_scales_large_repos() {
        assert_eq!(compute_budget(100, 0.2), 20);
    }

    #[test]
    fn compute_budget_returns_zero_for_empty_repos() {
        assert_eq!(compute_budget(0, 0.8), 0);
    }

    #[test]
    fn allocate_budget_short_circuits_when_budget_covers_all_candidates() {
        let candidates = candidates(&[
            ("file_page", 3),
            ("symbol_spotlight", 2),
            ("module_page", 1),
            ("api_contract", 1),
            ("infra_page", 1),
            ("scc_page", 1),
        ]);

        let allocation = allocate_budget(9, &candidates, &default_shares(), 100);

        assert_eq!(
            allocation,
            BucketAllocation {
                file_page: 3,
                symbol_spotlight: 2,
                module_page: 1,
                api_contract: 1,
                infra_page: 1,
                scc_page: 1,
            }
        );
    }

    #[test]
    fn allocate_budget_does_not_exceed_tight_budget_with_floors() {
        let candidates = candidates(&[
            ("file_page", 10),
            ("symbol_spotlight", 10),
            ("module_page", 10),
            ("api_contract", 10),
            ("infra_page", 10),
            ("scc_page", 10),
        ]);
        let shares = HashMap::new();

        let allocation = allocate_budget(2, &candidates, &shares, 100);

        assert!(allocation.total() <= 2);
    }

    #[test]
    fn allocate_budget_respects_per_bucket_floors_when_budget_allows() {
        let candidates = candidates(&[
            ("file_page", 10),
            ("symbol_spotlight", 10),
            ("module_page", 10),
            ("api_contract", 10),
            ("infra_page", 10),
            ("scc_page", 10),
        ]);
        let shares = HashMap::new();

        let allocation = allocate_budget(3, &candidates, &shares, 100);

        assert_eq!(allocation.api_contract, 1);
        assert_eq!(allocation.infra_page, 1);
        assert_eq!(allocation.scc_page, 1);
        assert!(allocation.total() >= 3);
    }

    #[test]
    fn allocate_budget_never_exceeds_budget() {
        let candidates = candidates(&[
            ("file_page", 10),
            ("symbol_spotlight", 10),
            ("module_page", 10),
            ("api_contract", 10),
            ("infra_page", 10),
            ("scc_page", 10),
        ]);
        let shares = default_shares();

        for budget in [0, 1, 2, 3, 4, 5, 10] {
            let allocation = allocate_budget(budget, &candidates, &shares, 100);

            assert!(allocation.total() <= budget);
        }
    }

    #[test]
    fn allocate_budget_spills_remaining_budget_to_file_page() {
        let candidates = candidates(&[
            ("file_page", 10),
            ("symbol_spotlight", 0),
            ("module_page", 0),
            ("api_contract", 0),
            ("infra_page", 0),
            ("scc_page", 0),
        ]);
        let mut shares = HashMap::new();
        shares.insert("file_page".to_string(), 0.2);

        let allocation = allocate_budget(5, &candidates, &shares, 100);

        assert_eq!(allocation.file_page, 5);
        assert_eq!(allocation.total(), 5);
    }

    #[test]
    fn bucket_allocation_total_sums_all_buckets() {
        let allocation = BucketAllocation {
            file_page: 1,
            symbol_spotlight: 2,
            module_page: 3,
            api_contract: 4,
            infra_page: 5,
            scc_page: 6,
        };

        assert_eq!(allocation.total(), 21);
    }
}
