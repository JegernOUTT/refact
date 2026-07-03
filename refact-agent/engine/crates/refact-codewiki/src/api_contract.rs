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

    let fastapi_receivers = fastapi_receiver_names(parsed);

    parsed.symbols.iter().any(|symbol| {
        symbol
            .decorators
            .iter()
            .any(|decorator| is_fastapi_route_decorator(decorator, &fastapi_receivers))
    })
}

fn fastapi_receiver_names(parsed: &ParsedFile) -> Vec<&str> {
    let mut names = vec!["app", "router"];

    for import in &parsed.imports {
        if import.module_path == "fastapi" {
            names.extend(import.imported_names.iter().filter_map(|name| {
                matches!(name.as_str(), "FastAPI" | "APIRouter").then_some(name.as_str())
            }));
        }
    }

    names
}

fn is_fastapi_route_decorator(decorator: &str, receivers: &[&str]) -> bool {
    let methods = ["get", "post", "put", "patch", "delete", "head", "options"];
    let head = decorator
        .trim_start_matches('@')
        .split('(')
        .next()
        .unwrap_or("")
        .trim();
    let Some((receiver, method)) = head.rsplit_once('.') else {
        return false;
    };
    let receiver = receiver.rsplit('.').next().unwrap_or(receiver);

    methods.contains(&method) && receivers.contains(&receiver)
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
            bases
                .iter()
                .any(|base| csharp_signature_has_base(&symbol.signature, base))
                || symbol.decorators.iter().any(|decorator| {
                    csharp_attribute_names(decorator).any(|attr| class_attrs.contains(&attr))
                })
        });

    has_class_signal
        || parsed
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == "method")
            .any(|symbol| {
                symbol.decorators.iter().any(|decorator| {
                    csharp_attribute_names(decorator).any(|attr| method_attrs.contains(&attr))
                })
            })
}

fn csharp_signature_has_base(signature: &str, expected: &str) -> bool {
    let inherited = signature.split(" where ").next().unwrap_or(signature);
    let Some((_, bases)) = inherited.split_once(':') else {
        return false;
    };

    bases.split(',').any(|base| {
        let token = base
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '.' && ch != '<');
        let token = token
            .split('<')
            .next()
            .unwrap_or(token)
            .rsplit('.')
            .next()
            .unwrap_or(token);
        token == expected
    })
}

fn csharp_attribute_names(decorator: &str) -> impl Iterator<Item = &str> {
    decorator
        .trim_start_matches('@')
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .filter_map(|part| {
            let attr = part.split('(').next().unwrap_or("").trim();
            if attr.is_empty() {
                None
            } else {
                Some(attr.rsplit('.').next().unwrap_or(attr))
            }
        })
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
    fn api_contract_python_fastapi_import_with_router_get_decorator_is_true() {
        let mut parsed = parsed_file("python");
        parsed.imports.push(Import {
            module_path: "fastapi".to_string(),
            imported_names: Vec::new(),
        });
        parsed.symbols.push(Symbol {
            kind: "function".to_string(),
            signature: "def list_users():".to_string(),
            decorators: vec!["@router.get(\"/users\")".to_string()],
        });

        assert!(is_fastapi_router(&parsed));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_python_fastapi_imported_receiver_route_decorator_is_true() {
        let mut parsed = parsed_file("python");
        parsed.imports.push(Import {
            module_path: "fastapi".to_string(),
            imported_names: vec!["FastAPI".to_string()],
        });
        parsed.symbols.push(Symbol {
            kind: "function".to_string(),
            signature: "def list_users():".to_string(),
            decorators: vec!["@FastAPI.get(\"/users\")".to_string()],
        });

        assert!(is_fastapi_router(&parsed));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_python_fastapi_http_exception_with_unrelated_decorator_is_false() {
        let mut parsed = parsed_file("python");
        parsed.imports.push(Import {
            module_path: "fastapi".to_string(),
            imported_names: vec!["HTTPException".to_string()],
        });
        parsed.symbols.push(Symbol {
            kind: "function".to_string(),
            signature: "def cached():".to_string(),
            decorators: vec!["@cache.get(\"users\")".to_string()],
        });

        assert!(!is_fastapi_router(&parsed));
        assert!(!detect_api_contract(&parsed));
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
    fn api_contract_csharp_controller_signature_is_true() {
        let mut parsed = parsed_file("csharp");
        parsed.symbols.push(Symbol {
            kind: "class".to_string(),
            signature: "public class UsersController : Microsoft.AspNetCore.Mvc.Controller"
                .to_string(),
            decorators: Vec::new(),
        });

        assert!(is_aspnet_controller(&parsed));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_csharp_generic_controllerbase_signature_is_true() {
        let mut parsed = parsed_file("csharp");
        parsed.symbols.push(Symbol {
            kind: "class".to_string(),
            signature: "public class UsersController : ControllerBase<User> where User : class"
                .to_string(),
            decorators: Vec::new(),
        });

        assert!(is_aspnet_controller(&parsed));
        assert!(detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_csharp_controllerhelper_class_name_is_false() {
        let mut parsed = parsed_file("csharp");
        parsed.symbols.push(Symbol {
            kind: "class".to_string(),
            signature: "public class FooControllerHelper".to_string(),
            decorators: Vec::new(),
        });

        assert!(!is_aspnet_controller(&parsed));
        assert!(!detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_csharp_controllerhelper_base_name_is_false() {
        let mut parsed = parsed_file("csharp");
        parsed.symbols.push(Symbol {
            kind: "class".to_string(),
            signature: "public class Foo : MyControllerHelper".to_string(),
            decorators: Vec::new(),
        });

        assert!(!is_aspnet_controller(&parsed));
        assert!(!detect_api_contract(&parsed));
    }

    #[test]
    fn api_contract_csharp_field_named_controller_is_false() {
        let mut parsed = parsed_file("csharp");
        parsed.symbols.push(Symbol {
            kind: "class".to_string(),
            signature: "public class Foo { private string Controller".to_string(),
            decorators: Vec::new(),
        });

        assert!(!is_aspnet_controller(&parsed));
        assert!(!detect_api_contract(&parsed));
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
    fn api_contract_csharp_combined_apicontroller_route_decorator_is_true() {
        let mut parsed = parsed_file("csharp");
        parsed.symbols.push(Symbol {
            kind: "class".to_string(),
            signature: "public class Users".to_string(),
            decorators: vec!["[ApiController, Route(\"users\")]".to_string()],
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
