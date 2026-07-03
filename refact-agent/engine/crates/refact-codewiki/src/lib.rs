pub mod api_contract;
pub mod budget;
pub mod claude_md;
pub mod decisions;
pub mod entry_points;
pub mod evolution;
pub mod gate;
pub mod graph_intelligence;
pub mod interlinking;
pub mod parsed_file;
pub mod rag;
pub mod selection_scoring;
pub mod semantic_match;
pub mod sources;
pub mod token_budget;
pub mod well_known;

pub use budget::{allocate, AllocatedPage};
pub use claude_md::{render_claude_md, ClaudeMdInput};
pub use decisions::{
    classify_evidence, corroborate, extract_decisions, Decision, DecisionGraph, DecisionSource,
    DecisionStatus, Provenance, Relation,
};
pub use evolution::build_decision_graph;
pub use gate::Verification;
pub use rag::{search_hybrid, ScoredDoc};
pub use selection_scoring::{
    select_pages, FileInfo, FileSelection, ModuleSelection, PageCandidate, PageKind, ParsedFile,
    SelectionInput, SymbolInfo,
};
pub use semantic_match::{find_related_decisions, DEFAULT_DEDUP_TAU};
pub use sources::extract_all;

#[cfg(test)]
mod tests {
    #[test]
    fn single_tau_used_by_evolution_and_semantic_match() {
        assert_eq!(
            crate::evolution::DEFAULT_DEDUP_TAU,
            crate::DEFAULT_DEDUP_TAU
        );
    }
}
