use crate::api_contract::{Import, ParsedFile, Symbol};
use tree_sitter::Node;

pub fn build_parsed_file(lang: &str, text: &str) -> ParsedFile {
    let parse_lang = match lang {
        "py" => "python",
        "cs" => "csharp",
        _ => lang,
    };
    let Some(tree) = refact_codegraph_parsers::parse_tree(parse_lang, text) else {
        return empty_file(parse_lang);
    };

    match parse_lang {
        "python" => build_python_file(parse_lang, tree.root_node(), text),
        "csharp" => build_csharp_file(parse_lang, tree.root_node(), text),
        _ => empty_file(lang),
    }
}

fn empty_file(lang: &str) -> ParsedFile {
    ParsedFile {
        language: lang.to_string(),
        is_api_contract: false,
        imports: Vec::new(),
        symbols: Vec::new(),
    }
}

fn build_python_file(lang: &str, root: Node, text: &str) -> ParsedFile {
    let mut parsed = empty_file(lang);
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        match node.kind() {
            "import_statement" => parsed.imports.extend(python_import_statement(node, text)),
            "import_from_statement" => {
                if let Some(import) = python_import_from_statement(node, text) {
                    parsed.imports.push(import);
                }
            }
            "decorated_definition" => {
                parsed.symbols.extend(python_decorated_symbols(node, text));
            }
            "function_definition" | "class_definition" => {
                if node.parent().map(|parent| parent.kind()) != Some("decorated_definition") {
                    if let Some(symbol) = python_symbol(node, text, Vec::new()) {
                        parsed.symbols.push(symbol);
                    }
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    parsed
}

fn build_csharp_file(lang: &str, root: Node, text: &str) -> ParsedFile {
    let mut parsed = empty_file(lang);
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        match node.kind() {
            "using_directive" => {
                if let Some(import) = csharp_using_directive(node, text) {
                    parsed.imports.push(import);
                }
            }
            "class_declaration" => {
                if let Some(symbol) = csharp_class_symbol(node, text) {
                    parsed.symbols.push(symbol);
                }
            }
            "method_declaration" => {
                if let Some(symbol) = csharp_method_symbol(node, text) {
                    parsed.symbols.push(symbol);
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    parsed
}

fn node_text<'a>(node: Node, text: &'a str) -> Option<&'a str> {
    node.utf8_text(text.as_bytes()).ok()
}

fn python_import_statement(node: Node, text: &str) -> Vec<Import> {
    let raw = node_text(node, text).unwrap_or("");
    let rest = raw.trim().strip_prefix("import").unwrap_or(raw).trim();
    rest.split(',')
        .filter_map(|part| {
            let name = part.trim().split_whitespace().next().unwrap_or("").trim();
            if name.is_empty() {
                None
            } else {
                Some(Import {
                    module_path: name.to_string(),
                    imported_names: Vec::new(),
                })
            }
        })
        .collect()
}

fn python_import_from_statement(node: Node, text: &str) -> Option<Import> {
    let raw = node_text(node, text)?.trim();
    let rest = raw.strip_prefix("from")?.trim();
    let (module, names) = rest.split_once(" import ")?;
    let imported_names = names
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(',')
        .filter_map(|part| {
            let name = part.trim().split_whitespace().next().unwrap_or("").trim();
            if name.is_empty() || name == "*" {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect();

    Some(Import {
        module_path: module.trim().to_string(),
        imported_names,
    })
}

fn python_decorated_symbols(node: Node, text: &str) -> Vec<Symbol> {
    let mut decorators = Vec::new();
    let mut symbols = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "decorator" => {
                if let Some(raw) = node_text(child, text) {
                    decorators.push(raw.trim().to_string());
                }
            }
            "function_definition" | "class_definition" => {
                if let Some(symbol) = python_symbol(child, text, decorators.clone()) {
                    symbols.push(symbol);
                }
            }
            _ => {}
        }
    }

    symbols
}

fn python_symbol(node: Node, text: &str, decorators: Vec<String>) -> Option<Symbol> {
    let raw = node_text(node, text)?.trim();
    let signature = raw.lines().next().unwrap_or("").trim().to_string();
    let kind = match node.kind() {
        "function_definition" => "function",
        "class_definition" => "class",
        _ => return None,
    };

    Some(Symbol {
        kind: kind.to_string(),
        signature,
        decorators,
    })
}

fn csharp_using_directive(node: Node, text: &str) -> Option<Import> {
    let raw = node_text(node, text)?.trim();
    let rest = raw.strip_prefix("using")?.trim();
    let module = rest
        .trim_end_matches(';')
        .split('=')
        .next_back()
        .unwrap_or("")
        .trim();
    if module.is_empty() {
        None
    } else {
        Some(Import {
            module_path: module.to_string(),
            imported_names: Vec::new(),
        })
    }
}

fn csharp_class_symbol(node: Node, text: &str) -> Option<Symbol> {
    let raw = node_text(node, text)?.trim();
    let signature = raw
        .find('{')
        .map(|idx| raw[..idx].trim().to_string())
        .unwrap_or_else(|| raw.lines().next().unwrap_or("").trim().to_string());
    let decorators = csharp_attribute_lists(node, text);

    Some(Symbol {
        kind: "class".to_string(),
        signature,
        decorators,
    })
}

fn csharp_method_symbol(node: Node, text: &str) -> Option<Symbol> {
    let raw = node_text(node, text)?.trim();
    let signature = raw
        .find('{')
        .or_else(|| raw.find("=>"))
        .map(|idx| raw[..idx].trim().to_string())
        .unwrap_or_else(|| raw.lines().next().unwrap_or("").trim().to_string());
    let decorators = csharp_attribute_lists(node, text);

    Some(Symbol {
        kind: "method".to_string(),
        signature,
        decorators,
    })
}

fn csharp_attribute_lists(node: Node, text: &str) -> Vec<String> {
    let mut decorators = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_list" {
            if let Some(raw) = node_text(child, text) {
                decorators.push(raw.trim().to_string());
            }
        }
    }

    if !decorators.is_empty() {
        return decorators;
    }

    let mut current = node.prev_sibling();

    while let Some(sibling) = current {
        if sibling.kind() == "attribute_list" {
            if let Some(raw) = node_text(sibling, text) {
                decorators.push(raw.trim().to_string());
            }
            current = sibling.prev_sibling();
        } else if node_text(sibling, text)
            .map(|value| value.trim().is_empty())
            .unwrap_or(false)
        {
            current = sibling.prev_sibling();
        } else {
            break;
        }
    }

    decorators.reverse();
    decorators
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_contract::detect_api_contract;

    #[test]
    fn parsed_file_python_fastapi_detects_api_contract() {
        let src = "from fastapi import APIRouter\nrouter = APIRouter()\n@router.get(\"/x\")\ndef h():\n    pass\n";
        let parsed = build_parsed_file("python", src);

        assert!(parsed
            .imports
            .iter()
            .any(|import| import.module_path == "fastapi"
                && import.imported_names.iter().any(|name| name == "APIRouter")));
        assert!(parsed.symbols.iter().any(|symbol| symbol.kind == "function"
            && symbol
                .decorators
                .iter()
                .any(|decorator| decorator.contains("@router.get"))));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn parsed_file_python_keeps_typed_function_signature() {
        let src = "def h(user_id: int) -> str:\n    return str(user_id)\n";
        let parsed = build_parsed_file("python", src);

        assert!(parsed
            .symbols
            .iter()
            .any(|symbol| symbol.kind == "function"
                && symbol.signature == "def h(user_id: int) -> str:"));
    }

    #[test]
    fn parsed_file_csharp_aspnet_detects_api_contract() {
        let src = "[ApiController]\npublic class Foo : ControllerBase {}";
        let parsed = build_parsed_file("csharp", src);

        assert!(parsed.symbols.iter().any(|symbol| symbol.kind == "class"
            && symbol.signature.contains("ControllerBase")
            && symbol
                .decorators
                .iter()
                .any(|decorator| decorator.contains("ApiController"))));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn parsed_file_csharp_aspnet_detects_method_route_attribute() {
        let src = "public class Foo {\n    [HttpGet(\"/users\")]\n    public IActionResult List() { return Ok(); }\n}";
        let parsed = build_parsed_file("csharp", src);

        assert!(parsed.symbols.iter().any(|symbol| symbol.kind == "method"
            && symbol.signature.contains("IActionResult List")
            && symbol
                .decorators
                .iter()
                .any(|decorator| decorator.contains("HttpGet"))));
        assert!(detect_api_contract(&parsed));
    }
}
