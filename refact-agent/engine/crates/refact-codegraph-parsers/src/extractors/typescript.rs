use tree_sitter::{Parser, Tree};

use crate::extractors::js_common::walk;
use crate::ir::{LangExtractor, RawRef, SymbolNode};

pub struct TypeScriptExtractor;

impl TypeScriptExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for TypeScriptExtractor {
    fn default() -> Self {
        TypeScriptExtractor
    }
}

impl LangExtractor for TypeScriptExtractor {
    fn language(&self) -> &'static str {
        "typescript"
    }

    fn extract(&self, tree: &Tree, source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let mut symbols = Vec::new();
        let mut refs = Vec::new();
        walk(
            tree.root_node(),
            source.as_bytes(),
            &[],
            &mut symbols,
            &mut refs,
            true,
        );
        (symbols, refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{EdgeKind, SymbolKind};

    fn extract(source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let tree = TypeScriptExtractor::parse(source).expect("parse ts");
        TypeScriptExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_classes_interfaces_and_type_aliases() {
        let src = "\
interface Shape {
    area(): number;
}

type Id = string;

class Circle implements Shape {
    area(): number {
        return compute();
    }
}

function compute(): number {
    return 0;
}
";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Shape".to_string()));
        assert!(paths.contains(&"Id".to_string()));
        assert!(paths.contains(&"Circle".to_string()));
        assert!(paths.contains(&"Circle::area".to_string()));
        assert!(paths.contains(&"compute".to_string()));
        let id = symbols.iter().find(|s| s.name() == "Id").unwrap();
        assert_eq!(id.kind, SymbolKind::TypeAlias);
        let circle = symbols.iter().find(|s| s.name() == "Circle").unwrap();
        assert_eq!(circle.this_class_derived_from, vec!["Shape".to_string()]);
        assert!(refs
            .iter()
            .any(|r| r.name == "compute" && r.from == "Circle::area"));
    }

    #[test]
    fn extracts_class_extends_heritage() {
        let src = "\
class Base {}
class Derived extends Base {}
";
        let (symbols, _refs) = extract(src);
        let derived = symbols.iter().find(|s| s.name() == "Derived").unwrap();
        assert_eq!(derived.this_class_derived_from, vec!["Base".to_string()]);
    }

    #[test]
    fn extracts_barrel_reexports_and_consumer_imports() {
        let (_barrel_symbols, barrel_refs) = extract("export { a } from './mod';\n");
        assert!(
            barrel_refs
                .iter()
                .any(|r| { r.kind == EdgeKind::Imports && r.name == "a" && r.from.is_empty() }),
            "barrel refs: {barrel_refs:?}"
        );

        let (_star_symbols, star_refs) = extract("export * from './mod';\n");
        assert!(
            star_refs
                .iter()
                .any(|r| { r.kind == EdgeKind::Imports && r.name == "./mod" && r.from.is_empty() }),
            "star refs: {star_refs:?}"
        );

        let (_consumer_symbols, consumer_refs) = extract("import { a } from './barrel';\n");
        assert!(
            consumer_refs.iter().any(|r| {
                r.kind == EdgeKind::Imports && r.name == "./barrel" && r.from.is_empty()
            }),
            "consumer refs: {consumer_refs:?}"
        );
        assert!(
            consumer_refs
                .iter()
                .any(|r| { r.kind == EdgeKind::Imports && r.name == "a" && r.from.is_empty() }),
            "consumer refs: {consumer_refs:?}"
        );
    }
}
