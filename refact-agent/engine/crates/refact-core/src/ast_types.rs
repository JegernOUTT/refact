use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use ropey::Rope;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify as ANotify;

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone, Eq, Hash)]
pub enum SymbolType {
    Module,
    StructDeclaration,
    TypeAlias,
    ClassFieldDeclaration,
    ImportDeclaration,
    VariableDefinition,
    FunctionDeclaration,
    CommentDefinition,
    FunctionCall,
    VariableUsage,
    Unknown,
}

impl fmt::Display for SymbolType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for SymbolType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "module" => SymbolType::Module,
            "struct_declaration" => SymbolType::StructDeclaration,
            "type_alias" => SymbolType::TypeAlias,
            "class_field_declaration" => SymbolType::ClassFieldDeclaration,
            "import_declaration" => SymbolType::ImportDeclaration,
            "variable_definition" => SymbolType::VariableDefinition,
            "function_declaration" => SymbolType::FunctionDeclaration,
            "comment_definition" => SymbolType::CommentDefinition,
            "function_call" => SymbolType::FunctionCall,
            "variable_usage" => SymbolType::VariableUsage,
            _ => SymbolType::Unknown,
        })
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AstUsage {
    pub targets_for_guesswork: Vec<String>,
    pub resolved_as: String,
    pub debug_hint: String,
    pub uline: usize,
}

#[derive(Serialize, Deserialize)]
pub struct AstDefinition {
    pub official_path: Vec<String>,
    pub symbol_type: SymbolType,
    pub usages: Vec<AstUsage>,
    pub resolved_type: String,
    pub this_is_a_class: String,
    pub this_class_derived_from: Vec<String>,
    pub cpath: String,
    pub decl_line1: usize,
    pub decl_line2: usize,
    pub body_line1: usize,
    pub body_line2: usize,
}

impl AstDefinition {
    pub fn path(&self) -> String {
        self.official_path.join("::")
    }

    pub fn path_drop0(&self) -> String {
        if self.official_path.len() > 3 {
            self.official_path
                .iter()
                .skip(1)
                .cloned()
                .collect::<Vec<String>>()
                .join("::")
        } else {
            self.official_path.join("::")
        }
    }

    pub fn name(&self) -> String {
        self.official_path.last().cloned().unwrap_or_default()
    }

    pub fn full_line1(&self) -> usize {
        self.decl_line1
    }

    pub fn full_line2(&self) -> usize {
        self.body_line2.max(self.decl_line2)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AstStatus {
    #[serde(skip)]
    pub astate_notify: Arc<ANotify>,
    #[serde(rename = "state")]
    pub astate: String,
    pub files_unparsed: usize,
    pub files_total: usize,
    pub ast_index_files_total: i32,
    pub ast_index_symbols_total: i32,
    pub ast_index_usages_total: i32,
    pub ast_max_files_hit: bool,
}

#[derive(Default, Debug)]
pub struct AstCounters {
    pub counter_defs: i32,
    pub counter_usages: i32,
    pub counter_docs: i32,
}

impl fmt::Debug for AstDefinition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let usages_paths: Vec<String> = self
            .usages
            .iter()
            .map(|link| format!("{:?}", link))
            .collect();
        let derived_from_paths: Vec<String> = self
            .this_class_derived_from
            .iter()
            .map(|link| format!("{:?}", link))
            .collect();

        let usages_str = if usages_paths.is_empty() {
            String::new()
        } else {
            format!(", usages: {}", usages_paths.join(" "))
        };

        let class_str = if self.this_is_a_class.is_empty() {
            String::new()
        } else {
            format!(", this_is_a_class: {}", self.this_is_a_class)
        };

        let derived_from_str = if derived_from_paths.is_empty() {
            String::new()
        } else {
            format!(", derived_from: {}", derived_from_paths.join(" "))
        };

        write!(
            f,
            "AstDefinition {{ {}{}{}{} }}",
            self.official_path.join("::"),
            usages_str,
            class_str,
            derived_from_str,
        )
    }
}

impl fmt::Debug for AstUsage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "U{{ {} {} }}",
            self.debug_hint,
            if !self.resolved_as.is_empty() {
                self.resolved_as.clone()
            } else {
                format!("guess {}", self.targets_for_guesswork.join(" "))
            }
        )
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct Document {
    pub doc_path: PathBuf,
    pub doc_text: Option<Rope>,
}

impl Document {
    pub fn new(doc_path: &PathBuf) -> Self {
        Self {
            doc_path: doc_path.clone(),
            doc_text: None,
        }
    }

    pub fn update_text(&mut self, text: &String) {
        self.doc_text = Some(Rope::from_str(text));
    }

    pub fn text_as_string(&self) -> Result<String, String> {
        if let Some(r) = &self.doc_text {
            return Ok(r.to_string());
        }
        Err(format!("no text loaded in {}", self.doc_path.display()))
    }

    pub fn does_text_look_good(&self) -> Result<(), String> {
        assert!(self.doc_text.is_some());
        let r = self.doc_text.as_ref().unwrap();

        let total_chars = r.chars().count();
        let total_lines = r.lines().count();
        let avg_line_length = total_chars / total_lines;
        if avg_line_length > 150 {
            return Err("generated, avg line length > 150".to_string());
        }

        let total_spaces = r.chars().filter(|x| x.is_whitespace()).count();
        let spaces_percentage = total_spaces as f32 / total_chars as f32;
        if total_lines >= 5 && spaces_percentage <= 0.05 {
            return Err(format!(
                "generated or compressed, {:.1}% spaces < 5%",
                100.0 * spaces_percentage
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn symbol_type_from_str_covers_all_kinds() {
        assert_eq!(SymbolType::from_str("module"), Ok(SymbolType::Module));
        assert_eq!(
            SymbolType::from_str("struct_declaration"),
            Ok(SymbolType::StructDeclaration)
        );
        assert_eq!(
            SymbolType::from_str("type_alias"),
            Ok(SymbolType::TypeAlias)
        );
        assert_eq!(
            SymbolType::from_str("function_declaration"),
            Ok(SymbolType::FunctionDeclaration)
        );
        assert_eq!(
            SymbolType::from_str("variable_usage"),
            Ok(SymbolType::VariableUsage)
        );
        assert_eq!(SymbolType::from_str("nonsense"), Ok(SymbolType::Unknown));
    }

    #[test]
    fn document_reports_and_updates_text() {
        let mut doc = Document::new(&std::path::PathBuf::from("/tmp/x.rs"));
        assert!(doc.text_as_string().is_err());
        doc.update_text(&"hello".to_string());
        assert_eq!(doc.text_as_string().unwrap(), "hello");
    }
}
