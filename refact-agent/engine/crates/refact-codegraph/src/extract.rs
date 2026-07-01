use refact_codegraph_parsers::{
    BashExtractor, CExtractor, CppExtractor, CSharpExtractor, EdgeKind, ElixirExtractor,
    GoExtractor, HaskellExtractor, JavaExtractor, JavaScriptExtractor, KotlinExtractor,
    LangExtractor, OcamlExtractor, PhpExtractor, PythonExtractor, RawRef, RubyExtractor,
    RustExtractor, ScalaExtractor, SwiftExtractor, SymbolNode, TypeScriptExtractor,
};

pub fn extract_symbols(lang: &str, text: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
    match lang {
        "rust" => RustExtractor::parse(text)
            .map(|t| RustExtractor.extract(&t, text))
            .unwrap_or_default(),
        "python" => PythonExtractor::parse(text)
            .map(|t| PythonExtractor.extract(&t, text))
            .unwrap_or_default(),
        "javascript" | "jsx" => JavaScriptExtractor::parse(text)
            .map(|t| JavaScriptExtractor.extract(&t, text))
            .unwrap_or_default(),
        "typescript" | "tsx" => TypeScriptExtractor::parse(text)
            .map(|t| TypeScriptExtractor.extract(&t, text))
            .unwrap_or_default(),
        "java" => JavaExtractor::parse(text)
            .map(|t| JavaExtractor.extract(&t, text))
            .unwrap_or_default(),
        "kotlin" => KotlinExtractor::parse(text)
            .map(|t| KotlinExtractor.extract(&t, text))
            .unwrap_or_default(),
        "c" => CExtractor::parse(text)
            .map(|t| CExtractor.extract(&t, text))
            .unwrap_or_default(),
        "cpp" => CppExtractor::parse(text)
            .map(|t| CppExtractor.extract(&t, text))
            .unwrap_or_default(),
        "bash" => BashExtractor::parse(text)
            .map(|t| BashExtractor.extract(&t, text))
            .unwrap_or_default(),
        "elixir" => ElixirExtractor::parse(text)
            .map(|t| ElixirExtractor.extract(&t, text))
            .unwrap_or_default(),
        "ocaml" => OcamlExtractor::parse(text)
            .map(|t| OcamlExtractor.extract(&t, text))
            .unwrap_or_default(),
        "haskell" => HaskellExtractor::parse(text)
            .map(|t| HaskellExtractor.extract(&t, text))
            .unwrap_or_default(),
        "go" => GoExtractor::parse(text)
            .map(|t| GoExtractor.extract(&t, text))
            .unwrap_or_default(),
        "csharp" => CSharpExtractor::parse(text)
            .map(|t| CSharpExtractor.extract(&t, text))
            .unwrap_or_default(),
        "ruby" => RubyExtractor::parse(text)
            .map(|t| RubyExtractor.extract(&t, text))
            .unwrap_or_default(),
        "php" => PhpExtractor::parse(text)
            .map(|t| PhpExtractor.extract(&t, text))
            .unwrap_or_default(),
        "swift" => SwiftExtractor::parse(text)
            .map(|t| SwiftExtractor.extract(&t, text))
            .unwrap_or_default(),
        "scala" => ScalaExtractor::parse(text)
            .map(|t| ScalaExtractor.extract(&t, text))
            .unwrap_or_default(),
        _ => (Vec::new(), Vec::new()),
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
