use std::sync::Arc;

use refact_core::ast_types::{AstDefinition, SymbolType};

pub fn format_symbols_from_defs(defs: &[Arc<AstDefinition>]) -> String {
    let symbols: Vec<String> = defs
        .iter()
        .filter(|x| {
            matches!(
                x.symbol_type,
                SymbolType::StructDeclaration
                    | SymbolType::TypeAlias
                    | SymbolType::FunctionDeclaration
            )
        })
        .map(|x| x.name())
        .collect();
    if symbols.is_empty() {
        String::new()
    } else {
        format!(" ({})", symbols.join(", "))
    }
}
