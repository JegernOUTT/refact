use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub const BONUS_ENTRY_POINT: f64 = 0.40;
pub const BONUS_HOTSPOT: f64 = 0.20;
pub const BONUS_INIT_PY_RE_EXPORTER: f64 = 0.15;
pub const BONUS_BETWEENNESS_BRIDGE: f64 = 0.10;
pub const PENALTY_TEST: f64 = 0.60;
pub const PENALTY_TRIVIAL_SIZE_BYTES: u64 = 4000;
pub const PENALTY_TRIVIAL_SYMBOL_CAP: usize = 4;
pub const PENALTY_TRIVIAL: f64 = 0.40;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub is_entry_point: bool,
    pub is_test: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub kind: String,
    pub visibility: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedFile {
    pub file_info: FileInfo,
    pub symbols: Vec<SymbolInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    File,
    Module,
    Scc,
    ApiContract,
    Infra,
}

impl PageKind {
    pub fn bucket(self) -> &'static str {
        match self {
            Self::File => "file_page",
            Self::Module => "module_page",
            Self::Scc => "scc_page",
            Self::ApiContract => "api_contract",
            Self::Infra => "infra_page",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageCandidate {
    pub id: String,
    pub kind: PageKind,
    pub score: f64,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileSelection {
    pub parsed: ParsedFile,
    pub pagerank: f64,
    pub betweenness: f64,
    pub is_hotspot: bool,
    pub kg_bonus: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleSelection {
    pub id: String,
    pub paths: Vec<String>,
    pub size: u64,
    pub cohesion: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SelectionInput {
    pub files: Vec<FileSelection>,
    pub modules: Vec<ModuleSelection>,
    pub sccs: Vec<Vec<String>>,
    pub api_contract_paths: Vec<String>,
    pub infra_paths: Vec<String>,
    pub min_module_size: u64,
}

pub fn normalize(value: f64, max_value: f64) -> f64 {
    if max_value <= 0.0 {
        0.0
    } else {
        (value / max_value).clamp(0.0, 1.0)
    }
}

pub fn score_file(
    parsed: &ParsedFile,
    pagerank: f64,
    betweenness: f64,
    max_pagerank: f64,
    max_betweenness: f64,
    is_hotspot: bool,
    kg_bonus: f64,
) -> f64 {
    let n_symbols = parsed.symbols.len();
    let file_info = &parsed.file_info;

    if n_symbols == 0 && !file_info.is_entry_point && !is_hotspot {
        return 0.0;
    }

    let mut base = 0.01 * (n_symbols.min(5) as f64)
        + normalize(pagerank, max_pagerank)
        + normalize(betweenness, max_betweenness) * 0.5;

    if file_info.is_entry_point {
        base += BONUS_ENTRY_POINT;
    }
    if is_hotspot {
        base += BONUS_HOTSPOT;
    }
    if betweenness > 0.0 && !file_info.is_entry_point {
        base += BONUS_BETWEENNESS_BRIDGE;
    }
    if file_info.path.ends_with("__init__.py") && n_symbols >= 2 {
        base += BONUS_INIT_PY_RE_EXPORTER;
    }

    base += kg_bonus;

    if file_info.is_test {
        base *= PENALTY_TEST;
    }
    if n_symbols <= PENALTY_TRIVIAL_SYMBOL_CAP
        && file_info.size_bytes < PENALTY_TRIVIAL_SIZE_BYTES
        && !file_info.is_entry_point
    {
        base *= PENALTY_TRIVIAL;
    }

    base
}

pub fn kind_weight(kind: &str) -> f64 {
    match kind {
        "function" => 1.0,
        "method" => 0.9,
        "class" => 1.0,
        "interface" => 0.9,
        "struct" => 0.8,
        "enum" => 0.7,
        "type_alias" => 0.5,
        "variable" => 0.3,
        "constant" => 0.4,
        _ => 0.5,
    }
}

pub fn score_symbol(sym: &SymbolInfo, file_pagerank: f64, max_pagerank: f64) -> f64 {
    if sym.visibility != "public" {
        return 0.0;
    }

    normalize(file_pagerank, max_pagerank) * kind_weight(&sym.kind)
}

pub fn score_module(size: u64, cohesion: f64, min_module_size: u64) -> f64 {
    if size < min_module_size {
        0.0
    } else {
        (size as f64).sqrt() * (0.5 + cohesion)
    }
}

pub fn score_scc(cycle_size: usize) -> f64 {
    if cycle_size <= 1 {
        0.0
    } else {
        cycle_size as f64
    }
}

pub fn score_api_contract(parsed: &ParsedFile) -> f64 {
    let n_public = parsed
        .symbols
        .iter()
        .filter(|sym| sym.visibility == "public")
        .count();
    let size_kb = 1.max(parsed.file_info.size_bytes / 1024);

    n_public as f64 + (size_kb as f64 / 10.0).min(5.0)
}

pub fn score_infra(parsed: &ParsedFile) -> f64 {
    let size_kb = 1.max(parsed.file_info.size_bytes / 1024);
    let name = parsed
        .file_info
        .path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(parsed.file_info.path.as_str());
    let boost = if matches!(name, "Dockerfile" | "Makefile" | "GNUmakefile") {
        0.5
    } else {
        0.0
    };

    (size_kb as f64 / 5.0).min(5.0) + boost
}

pub fn select_pages(input: &SelectionInput) -> Vec<PageCandidate> {
    let max_pagerank = input
        .files
        .iter()
        .map(|file| file.pagerank)
        .filter(|value| value.is_finite())
        .fold(0.0, f64::max);
    let max_betweenness = input
        .files
        .iter()
        .map(|file| file.betweenness)
        .filter(|value| value.is_finite())
        .fold(0.0, f64::max);
    let files_by_path: BTreeMap<&str, &FileSelection> = input
        .files
        .iter()
        .map(|file| (file.parsed.file_info.path.as_str(), file))
        .collect();
    let mut pages = Vec::new();

    for file in &input.files {
        push_page(
            &mut pages,
            PageCandidate {
                id: format!("file:{}", file.parsed.file_info.path),
                kind: PageKind::File,
                score: score_file(
                    &file.parsed,
                    file.pagerank,
                    file.betweenness,
                    max_pagerank,
                    max_betweenness,
                    file.is_hotspot,
                    file.kg_bonus,
                ),
                paths: vec![file.parsed.file_info.path.clone()],
            },
        );
    }

    for module in &input.modules {
        let paths = sorted_paths(&module.paths);
        push_page(
            &mut pages,
            PageCandidate {
                id: format!("module:{}", module.id),
                kind: PageKind::Module,
                score: score_module(module.size, module.cohesion, input.min_module_size),
                paths,
            },
        );
    }

    for paths in &input.sccs {
        let paths = sorted_paths(paths);
        if paths.is_empty() {
            continue;
        }
        push_page(
            &mut pages,
            PageCandidate {
                id: format!("scc:{}", paths.join("|")),
                kind: PageKind::Scc,
                score: score_scc(paths.len()),
                paths,
            },
        );
    }

    for path in sorted_paths(&input.api_contract_paths) {
        if let Some(file) = files_by_path.get(path.as_str()) {
            push_page(
                &mut pages,
                PageCandidate {
                    id: format!("api:{}", path),
                    kind: PageKind::ApiContract,
                    score: score_api_contract(&file.parsed),
                    paths: vec![path],
                },
            );
        }
    }

    for path in sorted_paths(&input.infra_paths) {
        if let Some(file) = files_by_path.get(path.as_str()) {
            push_page(
                &mut pages,
                PageCandidate {
                    id: format!("infra:{}", path),
                    kind: PageKind::Infra,
                    score: score_infra(&file.parsed),
                    paths: vec![path],
                },
            );
        }
    }

    pages.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    pages
}

fn sorted_paths(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| !path.is_empty())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn push_page(pages: &mut Vec<PageCandidate>, page: PageCandidate) {
    if page.score > 0.0 && !page.id.is_empty() {
        pages.push(page);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(
        path: &str,
        is_entry_point: bool,
        is_test: bool,
        size_bytes: u64,
        symbols: usize,
    ) -> ParsedFile {
        ParsedFile {
            file_info: FileInfo {
                path: path.to_string(),
                is_entry_point,
                is_test,
                size_bytes,
            },
            symbols: (0..symbols)
                .map(|_| SymbolInfo {
                    kind: "function".to_string(),
                    visibility: "public".to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn zero_symbol_non_entry_non_hotspot_scores_zero() {
        let file = parsed("src/empty.rs", false, false, 100, 0);

        assert_eq!(score_file(&file, 1.0, 1.0, 1.0, 1.0, false, 0.0), 0.0);
    }

    #[test]
    fn entry_point_bonus_raises_score() {
        let regular = parsed("src/main.rs", false, false, 5000, 5);
        let entry = parsed("src/main.rs", true, false, 5000, 5);

        let regular_score = score_file(&regular, 0.0, 0.0, 1.0, 1.0, false, 0.0);
        let entry_score = score_file(&entry, 0.0, 0.0, 1.0, 1.0, false, 0.0);

        assert!(entry_score > regular_score);
    }

    #[test]
    fn test_file_is_penalized() {
        let regular = parsed("src/lib.rs", false, false, 5000, 5);
        let test = parsed("src/lib_test.rs", false, true, 5000, 5);

        let regular_score = score_file(&regular, 1.0, 0.0, 1.0, 1.0, false, 0.0);
        let test_score = score_file(&test, 1.0, 0.0, 1.0, 1.0, false, 0.0);

        assert_eq!(test_score, regular_score * PENALTY_TEST);
    }

    #[test]
    fn trivial_small_file_is_penalized() {
        let non_trivial = parsed("src/lib.rs", false, false, 4000, 4);
        let trivial = parsed("src/lib.rs", false, false, 3999, 4);

        let non_trivial_score = score_file(&non_trivial, 1.0, 0.0, 1.0, 1.0, false, 0.0);
        let trivial_score = score_file(&trivial, 1.0, 0.0, 1.0, 1.0, false, 0.0);

        assert_eq!(trivial_score, non_trivial_score * PENALTY_TRIVIAL);
    }

    #[test]
    fn score_symbol_returns_zero_for_non_public() {
        let sym = SymbolInfo {
            kind: "function".to_string(),
            visibility: "private".to_string(),
        };

        assert_eq!(score_symbol(&sym, 1.0, 1.0), 0.0);
    }

    #[test]
    fn score_scc_handles_singletons_and_cycles() {
        assert_eq!(score_scc(1), 0.0);
        assert_eq!(score_scc(3), 3.0);
    }

    #[test]
    fn score_infra_boosts_top_level_dockerfile() {
        let dockerfile = parsed("Dockerfile", false, false, 1024, 0);
        let ordinary = parsed("ordinary", false, false, 1024, 0);

        assert_eq!(score_infra(&dockerfile), score_infra(&ordinary) + 0.5);
    }

    #[test]
    fn select_pages_emits_all_kinds() {
        let auth = parsed("src/auth.rs", true, false, 8192, 5);
        let dockerfile = parsed("Dockerfile", false, false, 1024, 0);
        let input = SelectionInput {
            files: vec![
                FileSelection {
                    parsed: auth.clone(),
                    pagerank: 2.0,
                    betweenness: 1.0,
                    is_hotspot: true,
                    kg_bonus: 0.3,
                },
                FileSelection {
                    parsed: dockerfile,
                    pagerank: 0.0,
                    betweenness: 0.0,
                    is_hotspot: false,
                    kg_bonus: 0.0,
                },
            ],
            modules: vec![ModuleSelection {
                id: "src".to_string(),
                paths: vec!["src/auth.rs".to_string(), "Dockerfile".to_string()],
                size: 20,
                cohesion: 0.5,
            }],
            sccs: vec![vec!["src/auth.rs".to_string(), "Dockerfile".to_string()]],
            api_contract_paths: vec!["src/auth.rs".to_string()],
            infra_paths: vec!["Dockerfile".to_string()],
            min_module_size: 10,
        };

        let pages = select_pages(&input);
        let kinds: BTreeSet<PageKind> = pages.iter().map(|page| page.kind).collect();
        let file_score = pages
            .iter()
            .find(|page| page.id == "file:src/auth.rs")
            .unwrap()
            .score;
        let old_file_score = score_file(&auth, 2.0, 1.0, 2.0, 1.0, true, 0.3);

        assert!(kinds.contains(&PageKind::File));
        assert!(kinds.contains(&PageKind::Module));
        assert!(kinds.contains(&PageKind::Scc));
        assert!(kinds.contains(&PageKind::ApiContract));
        assert!(kinds.contains(&PageKind::Infra));
        assert_eq!(file_score, old_file_score);
    }
}
