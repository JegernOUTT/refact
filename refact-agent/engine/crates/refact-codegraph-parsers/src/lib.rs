pub mod extractors;
pub mod frameworks;
pub mod heritage;
pub mod import_resolution;
pub mod ir;
pub mod resolver;

pub use extractors::{
    BashExtractor, CExtractor, CppExtractor, CSharpExtractor, ElixirExtractor, GoExtractor,
    HaskellExtractor, JavaExtractor, JavaScriptExtractor, KotlinExtractor, OcamlExtractor,
    PhpExtractor, PythonExtractor, RubyExtractor, RustExtractor, ScalaExtractor, SwiftExtractor,
    TypeScriptExtractor,
};
pub use frameworks::{FrameworkDetector, FrameworkRegistry};
pub use ir::{Edge, EdgeKind, LangExtractor, RawRef, SymbolKind, SymbolNode};
pub use resolver::{Resolution, ResolutionTier, Resolver};

pub fn normalize_lang(lang: &str) -> &str {
    match lang {
        "py" => "python",
        "jsx" => "javascript",
        "tsx" => "typescript",
        "cs" => "csharp",
        "rb" => "ruby",
        "c++" | "cc" | "cxx" => "cpp",
        _ => lang,
    }
}

pub fn parse_tree(lang: &str, text: &str) -> Option<tree_sitter::Tree> {
    let lang = normalize_lang(lang);
    match lang {
        "rust" => RustExtractor::parse(text),
        "python" => PythonExtractor::parse(text),
        "javascript" => JavaScriptExtractor::parse(text),
        "typescript" => TypeScriptExtractor::parse(text),
        "java" => JavaExtractor::parse(text),
        "kotlin" => KotlinExtractor::parse(text),
        "c" => CExtractor::parse(text),
        "cpp" => CppExtractor::parse(text),
        "bash" => BashExtractor::parse(text),
        "elixir" => ElixirExtractor::parse(text),
        "ocaml" => OcamlExtractor::parse(text),
        "haskell" => HaskellExtractor::parse(text),
        "go" => GoExtractor::parse(text),
        "csharp" => CSharpExtractor::parse(text),
        "ruby" => RubyExtractor::parse(text),
        "php" => PhpExtractor::parse(text),
        "swift" => SwiftExtractor::parse(text),
        "scala" => ScalaExtractor::parse(text),
        _ => None,
    }
}

pub fn resolve_refs(refs: &[RawRef], resolver: &Resolver) -> Vec<Edge> {
    refs.iter()
        .filter_map(|r| {
            resolver.resolve(&r.name).map(|res| Edge {
                src: r.from.clone(),
                dst: res.target,
                kind: r.kind,
                confidence: res.confidence,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_refs_builds_edges_with_confidence_from_resolver() {
        let mut resolver = Resolver::new();
        resolver.add_symbol("app::user::create");
        resolver.add_symbol("app::caller::run");

        let refs = vec![
            RawRef {
                from: "app::caller::run".to_string(),
                name: "app::user::create".to_string(),
                kind: EdgeKind::Calls,
                line: 10,
            },
            RawRef {
                from: "app::caller::run".to_string(),
                name: "nonexistent_symbol".to_string(),
                kind: EdgeKind::Calls,
                line: 11,
            },
        ];

        let edges = resolve_refs(&refs, &resolver);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].src, "app::caller::run");
        assert_eq!(edges[0].dst, "app::user::create");
        assert_eq!(edges[0].kind, EdgeKind::Calls);
        assert_eq!(edges[0].confidence, resolver::CONFIDENCE_EXACT);
    }
}
