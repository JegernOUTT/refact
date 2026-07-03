use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum SymbolKind {
    Module,
    Struct,
    TypeAlias,
    ClassField,
    Import,
    Variable,
    Function,
    Comment,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolNode {
    pub official_path: Vec<String>,
    pub kind: SymbolKind,
    pub cpath: String,
    pub decl_line1: usize,
    pub decl_line2: usize,
    pub body_line1: usize,
    pub body_line2: usize,
    pub this_is_a_class: String,
    pub this_class_derived_from: Vec<String>,
}

impl SymbolNode {
    pub fn double_colon_path(&self) -> String {
        self.official_path.join("::")
    }

    pub fn name(&self) -> String {
        self.official_path.last().cloned().unwrap_or_default()
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub enum EdgeKind {
    Imports,
    Calls,
    Inherits,
    Defines,
    RouteHandler,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub src: String,
    pub dst: String,
    pub kind: EdgeKind,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRef {
    pub from: String,
    pub name: String,
    pub kind: EdgeKind,
    pub line: usize,
}

pub trait LangExtractor {
    fn language(&self) -> &'static str;

    fn extract(&self, tree: &tree_sitter::Tree, source: &str) -> (Vec<SymbolNode>, Vec<RawRef>);
}
