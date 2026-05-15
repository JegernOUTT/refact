use std::collections::HashMap;
use std::sync::Arc;
use std::cell::RefCell;
use uuid::Uuid;
use crate::Document;
use crate::ast::treesitter::ast_instance_structs::SymbolInformation;
use crate::ast::treesitter::file_ast_markup::FileASTMarkup;

pub mod treesitter;

pub mod ast_db;
pub mod ast_parse_anything;
pub mod ast_structs;

pub mod parse_common;
pub mod parse_python;

pub fn lowlevel_file_markup(
    doc: &Document,
    symbols: &Vec<SymbolInformation>,
) -> Result<FileASTMarkup, String> {
    let t0 = std::time::Instant::now();
    assert!(doc.doc_text.is_some());
    let mut symbols4export: Vec<Arc<RefCell<SymbolInformation>>> = symbols
        .iter()
        .map(|s| Arc::new(RefCell::new(s.clone())))
        .collect();
    let guid_to_symbol: HashMap<Uuid, Arc<RefCell<SymbolInformation>>> = symbols4export
        .iter()
        .map(|s| (s.borrow().guid.clone(), s.clone()))
        .collect();
    fn recursive_path_of_guid(
        guid_to_symbol: &HashMap<Uuid, Arc<RefCell<SymbolInformation>>>,
        guid: &Uuid,
    ) -> String {
        return match guid_to_symbol.get(guid) {
            Some(x) => {
                let pname = if !x.borrow().name.is_empty() {
                    x.borrow().name.clone()
                } else {
                    x.borrow().guid.to_string()[..8].to_string()
                };
                let pp = recursive_path_of_guid(&guid_to_symbol, &x.borrow().parent_guid);
                format!("{}::{}", pp, pname)
            }
            None => {
                "UNK".to_string()
            }
        };
    }
    for s in symbols4export.iter_mut() {
        let symbol_path = recursive_path_of_guid(&guid_to_symbol, &s.borrow().guid);
        s.borrow_mut().symbol_path = symbol_path.clone();
    }
    symbols4export.sort_by(|a, b| {
        a.borrow()
            .symbol_path
            .len()
            .cmp(&b.borrow().symbol_path.len())
    });
    let x = FileASTMarkup {
        symbols_sorted_by_path_len: symbols4export.iter().map(|s| s.borrow().clone()).collect(),
    };
    let path_str = doc.doc_path.to_string_lossy().to_string();
    let n = 30usize;
    let short_path: String = path_str.chars().rev().take(n).collect::<String>().chars().rev().collect();
    let short_path = if short_path.len() == n { format!("...{}", short_path) } else { short_path };
    tracing::info!(
        "file_markup {:>4} symbols in {:.3}ms for {}",
        x.symbols_sorted_by_path_len.len(),
        t0.elapsed().as_secs_f32(),
        short_path,
    );
    Ok(x)
}
