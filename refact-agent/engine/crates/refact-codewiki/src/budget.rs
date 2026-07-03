use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::selection_scoring::{PageCandidate, PageKind};
use crate::token_budget::{estimate_tokens, items_within_budget};

pub const SMALL_REPO_THRESHOLD: u32 = 20;
pub const BUCKET_TYPES: [&str; 5] = [
    "file_page",
    "module_page",
    "api_contract",
    "infra_page",
    "scc_page",
];
pub const BUCKET_FLOOR: [(&str, u32); 5] = [
    ("file_page", 0),
    ("module_page", 0),
    ("api_contract", 1),
    ("infra_page", 1),
    ("scc_page", 1),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BucketAllocation {
    pub file_page: u32,
    pub module_page: u32,
    pub api_contract: u32,
    pub infra_page: u32,
    pub scc_page: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllocatedPage {
    pub page: PageCandidate,
    pub token_allowance: usize,
    pub estimated_tokens: usize,
}

impl BucketAllocation {
    pub fn total(&self) -> u32 {
        self.file_page + self.module_page + self.api_contract + self.infra_page + self.scc_page
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
        module_page: raw.get("module_page").copied().unwrap_or(0),
        api_contract: raw.get("api_contract").copied().unwrap_or(0),
        infra_page: raw.get("infra_page").copied().unwrap_or(0),
        scc_page: raw.get("scc_page").copied().unwrap_or(0),
    }
}

pub fn default_shares() -> HashMap<String, f64> {
    HashMap::from([
        ("file_page".to_string(), 0.70),
        ("module_page".to_string(), 0.12),
        ("api_contract".to_string(), 0.08),
        ("infra_page".to_string(), 0.05),
        ("scc_page".to_string(), 0.05),
    ])
}

pub fn allocate(pages: &[PageCandidate], budget_tokens: usize) -> Vec<AllocatedPage> {
    if pages.is_empty() || budget_tokens == 0 {
        return Vec::new();
    }

    let mut ranked = pages.to_vec();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));

    let candidates_per_bucket = candidates_per_bucket(&ranked);
    let n_files = candidates_per_bucket.get("file_page").copied().unwrap_or(0);
    let slot_budget = compute_budget(ranked.len() as u32, 1.0).min(budget_tokens as u32);
    let bucket_allocation = allocate_budget(
        slot_budget,
        &candidates_per_bucket,
        &default_shares(),
        n_files,
    );
    let selected = select_allocated_candidates(ranked, bucket_allocation);
    let (selected, used) = items_within_budget(selected, 0, budget_tokens, |page| {
        page_estimated_tokens(page).max(1)
    });
    if selected.is_empty() {
        return Vec::new();
    }

    let mut allocations: Vec<AllocatedPage> = selected
        .into_iter()
        .map(|page| {
            let estimated_tokens = page_estimated_tokens(&page).max(1);
            AllocatedPage {
                page,
                token_allowance: estimated_tokens,
                estimated_tokens,
            }
        })
        .collect();
    distribute_spill(&mut allocations, budget_tokens.saturating_sub(used));
    allocations
}

fn candidates_per_bucket(pages: &[PageCandidate]) -> HashMap<String, u32> {
    let mut counts = HashMap::new();
    for page in pages {
        *counts.entry(page.kind.bucket().to_string()).or_insert(0) += 1;
    }
    counts
}

fn select_allocated_candidates(
    pages: Vec<PageCandidate>,
    allocation: BucketAllocation,
) -> Vec<PageCandidate> {
    let mut remaining = HashMap::from([
        (PageKind::File.bucket(), allocation.file_page),
        (PageKind::Module.bucket(), allocation.module_page),
        (PageKind::Scc.bucket(), allocation.scc_page),
        (PageKind::ApiContract.bucket(), allocation.api_contract),
        (PageKind::Infra.bucket(), allocation.infra_page),
    ]);
    let mut selected = Vec::new();

    for page in pages {
        let bucket = page.kind.bucket();
        let Some(count) = remaining.get_mut(bucket) else {
            continue;
        };
        if *count == 0 {
            continue;
        }
        *count -= 1;
        selected.push(page);
    }

    selected
}

fn page_estimated_tokens(page: &PageCandidate) -> usize {
    let text = format!("{} {:?} {}", page.id, page.kind, page.paths.join(" "));
    estimate_tokens(&text)
}

fn distribute_spill(allocations: &mut [AllocatedPage], mut spill: usize) {
    if allocations.is_empty() || spill == 0 {
        return;
    }

    while spill > 0 {
        for allocation in allocations.iter_mut() {
            if spill == 0 {
                break;
            }
            allocation.token_allowance += 1;
            spill -= 1;
        }
    }
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
            module_page: 3,
            api_contract: 4,
            infra_page: 5,
            scc_page: 6,
        };

        assert_eq!(allocation.total(), 19);
    }

    fn page(id: &str, kind: PageKind, score: f64) -> PageCandidate {
        PageCandidate {
            id: id.to_string(),
            kind,
            score,
            paths: vec![id.to_string()],
        }
    }

    #[test]
    fn allocate_respects_total_budget() {
        let pages = vec![
            page("file:src/auth.rs", PageKind::File, 5.0),
            page("module:src", PageKind::Module, 4.0),
            page("api:src/auth.rs", PageKind::ApiContract, 3.0),
            page("infra:Dockerfile", PageKind::Infra, 2.0),
            page("scc:src/auth.rs|src/lib.rs", PageKind::Scc, 1.0),
        ];

        let allocated = allocate(&pages, 64);
        let total: usize = allocated.iter().map(|page| page.token_allowance).sum();

        assert!(!allocated.is_empty());
        assert!(total <= 64);
        assert!(allocated
            .iter()
            .all(|page| page.token_allowance >= page.estimated_tokens));
    }

    #[test]
    fn allocate_empty_is_empty() {
        assert!(allocate(&[], 100).is_empty());
    }
}
