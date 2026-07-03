use tree_sitter::{Parser, Tree};

use crate::extractors::js_common::walk;
use crate::ir::{LangExtractor, RawRef, SymbolNode};

pub struct JavaScriptExtractor;

impl JavaScriptExtractor {
    pub fn parse(source: &str) -> Option<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .ok()?;
        parser.parse(source, None)
    }
}

impl Default for JavaScriptExtractor {
    fn default() -> Self {
        JavaScriptExtractor
    }
}

impl LangExtractor for JavaScriptExtractor {
    fn language(&self) -> &'static str {
        "javascript"
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
            false,
        );
        (symbols, refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::SymbolKind;

    fn extract(source: &str) -> (Vec<SymbolNode>, Vec<RawRef>) {
        let tree = JavaScriptExtractor::parse(source).expect("parse js");
        JavaScriptExtractor.extract(&tree, source)
    }

    #[test]
    fn extracts_functions_classes_methods_and_heritage() {
        let src = "\
class Animal {
    speak() {}
}

class Dog extends Animal {
    speak() {
        bark();
    }
}

function bark() {}
";
        let (symbols, refs) = extract(src);
        let paths: Vec<String> = symbols.iter().map(|s| s.double_colon_path()).collect();
        assert!(paths.contains(&"Animal".to_string()));
        assert!(paths.contains(&"Dog".to_string()));
        assert!(paths.contains(&"Dog::speak".to_string()));
        assert!(paths.contains(&"bark".to_string()));
        let dog = symbols.iter().find(|s| s.name() == "Dog").unwrap();
        assert_eq!(dog.kind, SymbolKind::Struct);
        assert_eq!(dog.this_class_derived_from, vec!["Animal".to_string()]);
        assert!(refs
            .iter()
            .any(|r| r.name == "bark" && r.from == "Dog::speak"));
    }

    #[test]
    fn extracts_arrow_function_assigned_to_const() {
        let src = "const handler = () => { doThing(); };\nfunction doThing() {}\n";
        let (symbols, refs) = extract(src);
        assert!(symbols.iter().any(|s| s.name() == "handler"));
        assert!(refs
            .iter()
            .any(|r| r.name == "doThing" && r.from == "handler"));
    }
}
