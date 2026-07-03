pub const BONUS_ENTRY_POINT: f64 = 0.40;
pub const BONUS_HOTSPOT: f64 = 0.20;
pub const BONUS_INIT_PY_RE_EXPORTER: f64 = 0.15;
pub const BONUS_BETWEENNESS_BRIDGE: f64 = 0.10;
pub const PENALTY_TEST: f64 = 0.60;
pub const PENALTY_TRIVIAL_SIZE_BYTES: u64 = 4000;
pub const PENALTY_TRIVIAL_SYMBOL_CAP: usize = 4;
pub const PENALTY_TRIVIAL: f64 = 0.40;

pub struct FileInfo {
    pub path: String,
    pub is_entry_point: bool,
    pub is_test: bool,
    pub size_bytes: u64,
}

pub struct SymbolInfo {
    pub kind: String,
    pub visibility: String,
}

pub struct ParsedFile {
    pub file_info: FileInfo,
    pub symbols: Vec<SymbolInfo>,
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
}
