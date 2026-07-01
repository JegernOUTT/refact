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

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, symbol_type: SymbolType) -> Arc<AstDefinition> {
        Arc::new(AstDefinition {
            official_path: vec!["file".to_string(), name.to_string()],
            symbol_type,
            usages: Vec::new(),
            resolved_type: String::new(),
            this_is_a_class: String::new(),
            this_class_derived_from: Vec::new(),
            cpath: String::new(),
            decl_line1: 1,
            decl_line2: 1,
            body_line1: 1,
            body_line2: 1,
        })
    }

    #[test]
    fn formats_only_declaration_symbols() {
        let defs = vec![
            def("Foo", SymbolType::StructDeclaration),
            def("bar", SymbolType::FunctionDeclaration),
            def("x", SymbolType::VariableUsage),
        ];
        assert_eq!(format_symbols_from_defs(&defs), " (Foo, bar)");
        assert_eq!(format_symbols_from_defs(&[]), "");
    }
}
