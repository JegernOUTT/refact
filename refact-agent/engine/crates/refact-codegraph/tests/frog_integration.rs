use std::path::PathBuf;

use refact_codegraph::Store;

fn frog_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/emergency_frog_situation")
}

fn index_frog_suite() -> (Store, String, String) {
    let dir = frog_dir();
    let store = Store::open_in_memory().unwrap();
    let frog_path = dir.join("frog.py").to_string_lossy().to_string();
    let jump_path = dir
        .join("jump_to_conclusions.py")
        .to_string_lossy()
        .to_string();
    for path in [&frog_path, &jump_path] {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        store.index_file_graph(path, &text, "python").unwrap();
    }
    store.connect_usages().unwrap();
    (store, frog_path, jump_path)
}

#[test]
fn frog_suite_extracts_expected_symbols() {
    let (store, _frog_path, _jump_path) = index_frog_suite();
    let symbols: Vec<String> = store
        .all_symbols()
        .unwrap()
        .into_iter()
        .map(|(p, _)| p)
        .collect();

    for expected in [
        "tests/emergency_frog_situation/frog.py::Frog",
        "tests/emergency_frog_situation/frog.py::Frog::__init__",
        "tests/emergency_frog_situation/frog.py::Frog::bounce_off_banks",
        "tests/emergency_frog_situation/frog.py::Frog::jump",
        "tests/emergency_frog_situation/frog.py::Frog::croak",
        "tests/emergency_frog_situation/frog.py::Frog::swim",
        "tests/emergency_frog_situation/frog.py::AlternativeFrog",
        "tests/emergency_frog_situation/frog.py::AlternativeFrog::alternative_jump",
        "tests/emergency_frog_situation/frog.py::standalone_jumping_function",
        "tests/emergency_frog_situation/jump_to_conclusions.py::draw_hello_frog",
        "tests/emergency_frog_situation/jump_to_conclusions.py::main_loop",
    ] {
        assert!(
            symbols.iter().any(|s| s == expected),
            "expected symbol {expected} missing; got {symbols:?}"
        );
    }
}

#[test]
fn frog_suite_resolves_intra_file_calls() {
    let (store, frog_path, _jump_path) = index_frog_suite();
    let usages: Vec<String> = store
        .doc_usages(&frog_path)
        .unwrap()
        .into_iter()
        .map(|(_, name)| name)
        .collect();
    assert!(
        usages.iter().any(|n| n.ends_with("::bounce_off_banks")),
        "Frog::jump/swim should call bounce_off_banks; got {usages:?}"
    );
}

#[test]
fn frog_suite_end_to_end_agent_surface() {
    // Programmatic substitute for the manual engine smoke test: exercises every
    // agent-facing codegraph capability on a real multi-file Python fixture.
    let (store, frog_path, _jump_path) = index_frog_suite();

    // symbol_def / @definition path: definitions() resolves a method by name.
    let defs = refact_codegraph::facade::definitions(&store, "Frog::jump").unwrap();
    assert!(
        defs.iter().any(|d| d.name() == "jump"),
        "definitions(Frog::jump) should resolve the method; got {:?}",
        defs.iter().map(|d| d.path()).collect::<Vec<_>>()
    );

    // @definition fuzzy completion path.
    let fuzzy = refact_codegraph::facade::definition_paths_fuzzy(&store, "jump", 50).unwrap();
    assert!(
        fuzzy.iter().any(|p| p.contains("jump")),
        "fuzzy search for 'jump' should match; got {fuzzy:?}"
    );

    // cat / @tree path: doc_defs lists the file's definitions with line ranges.
    let doc_defs = refact_codegraph::facade::doc_defs(&store, &frog_path).unwrap();
    assert!(
        doc_defs
            .iter()
            .any(|d| d.name() == "Frog" && d.full_line1() >= 1),
        "doc_defs should list the Frog class with a valid line; got {} defs",
        doc_defs.len()
    );

    // fetch_counters (ast_db parity surface).
    let counters = refact_codegraph::facade::fetch_counters(&store).unwrap();
    assert!(counters.counter_defs >= 8, "counters: {counters:?}");
    assert!(
        counters.counter_docs >= 2,
        "two files indexed: {counters:?}"
    );

    // type_hierarchy must succeed (string output, possibly empty).
    let _hierarchy = refact_codegraph::facade::type_hierarchy(&store, "").unwrap();

    // @search / semantic_search path: hybrid FTS+graph retrieval finds the fixture.
    let hits = refact_codegraph::retrieval::search_hybrid(&store, "jump frog", 10).unwrap();
    assert!(
        hits.iter().any(|h| h.path.contains("frog")),
        "hybrid search for 'jump frog' should surface a frog file; got {:?}",
        hits.iter().map(|h| h.path.clone()).collect::<Vec<_>>()
    );

    // codegraph_overview tool path: analytics over the real graph.
    let overview = refact_codegraph::analytics::compute_overview(&store, 10).unwrap();
    assert!(overview.node_count >= 8, "overview: {overview:?}");
    assert!(
        overview.edge_count >= 1,
        "graph should have call/inherit edges: {overview:?}"
    );
}

#[test]
fn frog_suite_resolves_cross_file_calls() {
    let (store, _frog_path, jump_path) = index_frog_suite();
    let usages: Vec<String> = store
        .doc_usages(&jump_path)
        .unwrap()
        .into_iter()
        .map(|(_, name)| name)
        .collect();
    assert!(
        usages.iter().any(|n| n.ends_with("::draw_hello_frog")),
        "main_loop should call draw_hello_frog; got {usages:?}"
    );
    assert!(
        usages.iter().any(|n| n.ends_with("::jump")),
        "main_loop should call Frog::jump across files; got {usages:?}"
    );
}
