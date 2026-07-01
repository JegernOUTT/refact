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
        );
        (symbols, refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::SymbolKind;

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
}
