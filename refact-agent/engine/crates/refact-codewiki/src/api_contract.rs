pub struct Import {
    pub module_path: String,
    pub imported_names: Vec<String>,
}

pub struct Symbol {
    pub kind: String,
    pub signature: String,
    pub decorators: Vec<String>,
}

pub struct ParsedFile {
    pub language: String,
    pub is_api_contract: bool,
    pub imports: Vec<Import>,
    pub symbols: Vec<Symbol>,
}

pub fn is_fastapi_router(parsed: &ParsedFile) -> bool {
    let imports_fastapi = parsed.imports.iter().any(|import| {
        import.module_path == "fastapi" || import.module_path.starts_with("fastapi.")
    });

    if !imports_fastapi {
        return false;
    }

    if parsed.imports.iter().any(|import| {
        import
            .imported_names
            .iter()
            .any(|name| name == "APIRouter" || name == "FastAPI")
    }) {
        return true;
    }

    let methods = ["get", "post", "put", "patch", "delete", "head", "options"];

    parsed.symbols.iter().any(|symbol| {
        symbol.decorators.iter().any(|decorator| {
            let head = decorator
                .trim_start_matches('@')
                .split('(')
                .next()
                .unwrap_or("");

            head.contains('.')
                && head
                    .rsplit('.')
                    .next()
                    .map(|segment| methods.contains(&segment))
                    .unwrap_or(false)
        })
    })
}

pub fn is_aspnet_controller(parsed: &ParsedFile) -> bool {
    let bases = ["ControllerBase", "Controller", "ApiController"];
    let class_attrs = ["ApiController", "Route"];
    let method_attrs = [
        "Route",
        "HttpGet",
        "HttpPost",
        "HttpPut",
        "HttpDelete",
        "HttpPatch",
    ];

    let has_class_signal = parsed
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == "class")
        .any(|symbol| {
            bases.iter().any(|base| symbol.signature.contains(base))
                || symbol.decorators.iter().any(|decorator| {
                    csharp_attribute_name(decorator)
                        .map(|attr| class_attrs.contains(&attr))
                        .unwrap_or(false)
                })
        });

    has_class_signal
        || parsed
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == "method")
            .any(|symbol| {
                symbol.decorators.iter().any(|decorator| {
                    csharp_attribute_name(decorator)
                        .map(|attr| method_attrs.contains(&attr))
                        .unwrap_or(false)
                })
            })
}

fn csharp_attribute_name(decorator: &str) -> Option<&str> {
    let attr = decorator
        .trim_start_matches('@')
        .trim_start_matches('[')
        .split('(')
        .next()
        .unwrap_or("")
        .trim_end_matches(']')
        .trim();

    if attr.is_empty() {
        None
    } else {
        Some(attr.rsplit('.').next().unwrap_or(attr))
    }
}

pub fn detect_api_contract(parsed: &ParsedFile) -> bool {
    match parsed.language.as_str() {
        "python" => is_fastapi_router(parsed),
        "csharp" => is_aspnet_controller(parsed),
        _ => false,
    }
}

pub fn count_newly_flagged(files: &[ParsedFile]) -> usize {
    files
        .iter()
        .filter(|file| !file.is_api_contract && detect_api_contract(file))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed_file(language: &str) -> ParsedFile {
        ParsedFile {
            language: language.to_string(),
            is_api_contract: false,
            imports: Vec::new(),
            symbols: Vec::new(),
        }
    }

    #[test]
    fn api_contract_python_fastapi_import_with_apirouter_is_true() {
        let parsed = ParsedFile {
            language: "python".to_string(),
            is_api_contract: false,
            imports: vec![Import {
                module_path: "fastapi".to_string(),
                imported_names: vec!["APIRouter".to_string()],
            }],
            symbols: Vec::new(),
        };

        assert!(is_fastapi_router(&parsed));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_python_decorator_without_fastapi_import_is_false() {
        let mut parsed = parsed_file("python");
        parsed.symbols.push(Symbol {
            kind: "function".to_string(),
            signature: "def x():".to_string(),
            decorators: vec!["@router.get(\"/x\")".to_string()],
        });

        assert!(!is_fastapi_router(&parsed));
        assert!(!detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_python_fastapi_import_with_app_post_decorator_is_true() {
        let mut parsed = parsed_file("python");
        parsed.imports.push(Import {
            module_path: "fastapi".to_string(),
            imported_names: Vec::new(),
        });
        parsed.symbols.push(Symbol {
            kind: "function".to_string(),
            signature: "def create():".to_string(),
            decorators: vec!["@app.post(\"/x\")".to_string()],
        });

        assert!(is_fastapi_router(&parsed));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_csharp_controllerbase_signature_is_true() {
        let mut parsed = parsed_file("csharp");
        parsed.symbols.push(Symbol {
            kind: "class".to_string(),
            signature: "public class UsersController : ControllerBase".to_string(),
            decorators: Vec::new(),
        });

        assert!(is_aspnet_controller(&parsed));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_csharp_apicontroller_decorator_is_true() {
        let mut parsed = parsed_file("csharp");
        parsed.symbols.push(Symbol {
            kind: "class".to_string(),
            signature: "public class Users".to_string(),
            decorators: vec!["[ApiController]".to_string()],
        });

        assert!(is_aspnet_controller(&parsed));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_csharp_method_route_decorator_is_true() {
        let mut parsed = parsed_file("csharp");
        parsed.symbols.push(Symbol {
            kind: "class".to_string(),
            signature: "public class Users".to_string(),
            decorators: Vec::new(),
        });
        parsed.symbols.push(Symbol {
            kind: "method".to_string(),
            signature: "public IActionResult List()".to_string(),
            decorators: vec!["[HttpGet(\"/users\")]".to_string()],
        });

        assert!(is_aspnet_controller(&parsed));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_plain_code_file_is_false() {
        let parsed = parsed_file("rust");

        assert!(!detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_count_newly_flagged_skips_already_flagged_files() {
        let already_flagged = ParsedFile {
            language: "python".to_string(),
            is_api_contract: true,
            imports: vec![Import {
                module_path: "fastapi".to_string(),
                imported_names: vec!["APIRouter".to_string()],
            }],
            symbols: Vec::new(),
        };
        let newly_flagged = ParsedFile {
            language: "csharp".to_string(),
            is_api_contract: false,
            imports: Vec::new(),
            symbols: vec![Symbol {
                kind: "class".to_string(),
                signature: "public class UsersController : ControllerBase".to_string(),
                decorators: Vec::new(),
            }],
        };
        let plain = parsed_file("rust");

        assert_eq!(
            count_newly_flagged(&[already_flagged, newly_flagged, plain]),
            1
        );
    }
}
