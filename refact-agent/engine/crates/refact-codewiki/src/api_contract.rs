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
