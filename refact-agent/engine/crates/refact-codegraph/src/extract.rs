use std::fmt;

use refact_codegraph_parsers::{
    BashExtractor, CExtractor, CppExtractor, CSharpExtractor, EdgeKind, ElixirExtractor,
    GoExtractor, HaskellExtractor, JavaExtractor, JavaScriptExtractor, KotlinExtractor,
    LangExtractor, OcamlExtractor, PhpExtractor, PythonExtractor, RawRef, RubyExtractor,
    RustExtractor, ScalaExtractor, SwiftExtractor, SymbolNode, TypeScriptExtractor,
};
use tree_sitter::Tree;

pub const PARSER_UNAVAILABLE: &str = "tree-sitter parser unavailable";
pub const SYNTAX_ERRORS: &str = "tree-sitter reported syntax errors";

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractError {
    pub lang: String,
    pub reason: String,
    pub symbols: Vec<SymbolNode>,
    pub refs: Vec<RawRef>,
}

impl ExtractError {
    pub fn recovered_symbol_count(&self) -> usize {
        self.symbols.len()
    }
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "codegraph extract {}: {} ({} symbols recovered)",
            self.lang,
            self.reason,
            self.symbols.len()
        )
    }
}

impl std::error::Error for ExtractError {}

pub type ExtractResult = Result<(Vec<SymbolNode>, Vec<RawRef>), ExtractError>;

fn extract_with<E: LangExtractor>(
    lang: &str,
    text: &str,
    parse: fn(&str) -> Option<Tree>,
    extractor: E,
) -> ExtractResult {
    let Some(tree) = parse(text) else {
        return Err(ExtractError {
            lang: lang.to_string(),
            reason: PARSER_UNAVAILABLE.to_string(),
            symbols: Vec::new(),
            refs: Vec::new(),
        });
    };
    let (symbols, refs) = extractor.extract(&tree, text);
    if tree.root_node().has_error() {
        return Err(ExtractError {
            lang: lang.to_string(),
            reason: SYNTAX_ERRORS.to_string(),
            symbols,
            refs,
        });
    }
    Ok((symbols, refs))
}

pub fn extract_symbols(lang: &str, text: &str) -> ExtractResult {
    match lang {
        "rust" => extract_with(lang, text, RustExtractor::parse, RustExtractor),
        "python" => extract_with(lang, text, PythonExtractor::parse, PythonExtractor),
        "javascript" | "jsx" => {
            extract_with(lang, text, JavaScriptExtractor::parse, JavaScriptExtractor)
        }
        "typescript" | "tsx" => {
            extract_with(lang, text, TypeScriptExtractor::parse, TypeScriptExtractor)
        }
        "java" => extract_with(lang, text, JavaExtractor::parse, JavaExtractor),
        "kotlin" => extract_with(lang, text, KotlinExtractor::parse, KotlinExtractor),
        "c" => extract_with(lang, text, CExtractor::parse, CExtractor),
        "cpp" => extract_with(lang, text, CppExtractor::parse, CppExtractor),
        "bash" => extract_with(lang, text, BashExtractor::parse, BashExtractor),
        "elixir" => extract_with(lang, text, ElixirExtractor::parse, ElixirExtractor),
        "ocaml" => extract_with(lang, text, OcamlExtractor::parse, OcamlExtractor),
        "haskell" => extract_with(lang, text, HaskellExtractor::parse, HaskellExtractor),
        "go" => extract_with(lang, text, GoExtractor::parse, GoExtractor),
        "csharp" => extract_with(lang, text, CSharpExtractor::parse, CSharpExtractor),
        "ruby" => extract_with(lang, text, RubyExtractor::parse, RubyExtractor),
        "php" => extract_with(lang, text, PhpExtractor::parse, PhpExtractor),
        "swift" => extract_with(lang, text, SwiftExtractor::parse, SwiftExtractor),
        "scala" => extract_with(lang, text, ScalaExtractor::parse, ScalaExtractor),
        _ => Ok((Vec::new(), Vec::new())),
    }
}

pub fn edge_kind_str(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Imports => "imports",
        EdgeKind::Calls => "calls",
        EdgeKind::Inherits => "inherits",
        EdgeKind::Defines => "defines",
        EdgeKind::RouteHandler => "route_handler",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_file_without_symbols_is_not_a_failure() {
        let (symbols, refs) = extract_symbols("rust", "// nothing here\n").unwrap();
        assert!(symbols.is_empty());
        assert!(refs.is_empty());
    }

    #[test]
    fn malformed_source_is_reported_as_parse_failure_not_empty_success() {
        let malformed = "fn broken( { let x = ; ) } struct !!!\n";
        let err = extract_symbols("rust", malformed)
            .expect_err("malformed source must not report as a clean empty extraction");
        assert_eq!(err.lang, "rust");
        assert_eq!(err.reason, SYNTAX_ERRORS);
        assert!(err.to_string().contains("codegraph extract rust"));
    }

    #[test]
    fn failure_and_genuine_emptiness_are_distinguishable() {
        let empty = extract_symbols("python", "# just a comment\n");
        let broken = extract_symbols("python", "def (((:\n    return ]]]\n");
        assert!(empty.is_ok());
        assert!(broken.is_err());
        assert_eq!(empty.unwrap().0.len(), 0);
    }

    #[test]
    fn unsupported_language_stays_a_clean_empty_result() {
        let (symbols, refs) = extract_symbols("brainfuck", "+++[->+++<]\n").unwrap();
        assert!(symbols.is_empty());
        assert!(refs.is_empty());
    }

    #[test]
    fn parse_failure_still_recovers_partial_symbols() {
        let partly_broken = "fn good() {}\nfn broken( { ; ) }\n";
        let err = extract_symbols("rust", partly_broken).expect_err("must flag the broken half");
        assert!(
            err.recovered_symbol_count() > 0,
            "partial symbols must survive so indexing stays useful"
        );
    }
}
