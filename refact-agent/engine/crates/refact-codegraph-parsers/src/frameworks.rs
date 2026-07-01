use tree_sitter::Node;

use crate::ir::{Edge, SymbolNode};

pub trait FrameworkDetector: Send + Sync {
    fn name(&self) -> &'static str;

    fn detect_routes(&self, symbols: &[SymbolNode], source: &str) -> Vec<Edge>;
}

#[derive(Default)]
pub struct FrameworkRegistry {
    detectors: Vec<Box<dyn FrameworkDetector>>,
}

impl FrameworkRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, detector: Box<dyn FrameworkDetector>) {
        self.detectors.push(detector);
    }

    pub fn detect_all(&self, symbols: &[SymbolNode], source: &str) -> Vec<Edge> {
        self.detectors
            .iter()
            .flat_map(|d| d.detect_routes(symbols, source))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.detectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.detectors.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRoute {
    pub method: String,
    pub path: String,
    pub handler: String,
}

impl DetectedRoute {
    pub fn label(&self) -> String {
        format!("{} {}", self.method.to_uppercase(), self.path)
    }
}

const HTTP_METHODS: &[&str] = &[
    "get", "post", "put", "delete", "patch", "head", "options", "route", "all", "use",
];

fn node_text<'a>(node: Node, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

fn strip_quotes(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

fn first_string_arg(args: Node, bytes: &[u8]) -> Option<String> {
    let mut cursor = args.walk();
    for child in args.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "string" | "string_literal" | "interpreted_string_literal"
        ) {
            return Some(strip_quotes(node_text(child, bytes)));
        }
    }
    None
}

fn last_identifier_arg(args: Node, bytes: &[u8]) -> Option<String> {
    let mut cursor = args.walk();
    let mut found = None;
    for child in args.named_children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "member_expression") {
            found = Some(
                node_text(child, bytes)
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_string(),
            );
        }
    }
    found
}

/// Detect HTTP route -> handler associations for the given language.
pub fn detect_routes(lang: &str, source: &str) -> Vec<DetectedRoute> {
    let Some(tree) = crate::parse_tree(lang, source) else {
        return vec![];
    };
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    match lang {
        "python" => detect_python(tree.root_node(), bytes, &mut out),
        "javascript" | "jsx" | "typescript" | "tsx" => {
            detect_express(tree.root_node(), bytes, &mut out)
        }
        _ => {}
    }
    out
}

fn detect_python(node: Node, bytes: &[u8], out: &mut Vec<DetectedRoute>) {
    if node.kind() == "decorated_definition" {
        let handler = node
            .child_by_field_name("definition")
            .and_then(|def| def.child_by_field_name("name"))
            .map(|n| node_text(n, bytes).to_string());
        if let Some(handler) = handler {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() != "decorator" {
                    continue;
                }
                if let Some(route) = python_decorator_route(child, bytes, &handler) {
                    out.push(route);
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        detect_python(child, bytes, out);
    }
}

fn python_decorator_route(decorator: Node, bytes: &[u8], handler: &str) -> Option<DetectedRoute> {
    let mut cursor = decorator.walk();
    let call = decorator
        .named_children(&mut cursor)
        .find(|c| c.kind() == "call")?;
    let func = call.child_by_field_name("function")?;
    if func.kind() != "attribute" {
        return None;
    }
    let method = node_text(func.child_by_field_name("attribute")?, bytes).to_lowercase();
    if !HTTP_METHODS.contains(&method.as_str()) {
        return None;
    }
    let args = call.child_by_field_name("arguments")?;
    let path = first_string_arg(args, bytes).unwrap_or_default();
    Some(DetectedRoute {
        method: if method == "route" {
            "ANY".to_string()
        } else {
            method
        },
        path,
        handler: handler.to_string(),
    })
}

fn detect_express(node: Node, bytes: &[u8], out: &mut Vec<DetectedRoute>) {
    if node.kind() == "call_expression" {
        if let Some(func) = node.child_by_field_name("function") {
            if func.kind() == "member_expression" {
                if let Some(prop) = func.child_by_field_name("property") {
                    let method = node_text(prop, bytes).to_lowercase();
                    if HTTP_METHODS.contains(&method.as_str()) {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            let path = first_string_arg(args, bytes);
                            let handler = last_identifier_arg(args, bytes);
                            if let (Some(path), Some(handler)) = (path, handler) {
                                out.push(DetectedRoute {
                                    method,
                                    path,
                                    handler,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        detect_express(child, bytes, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_fastapi_flask_routes() {
        let src = "\
@app.get(\"/users\")
def list_users():
    pass

@router.post(\"/users\")
def create_user():
    pass

@app.route(\"/health\")
def health():
    pass
";
        let routes = detect_routes("python", src);
        assert!(
            routes
                .iter()
                .any(|r| r.method == "get" && r.path == "/users" && r.handler == "list_users"),
            "got {routes:?}"
        );
        assert!(
            routes
                .iter()
                .any(|r| r.method == "post" && r.handler == "create_user"),
            "got {routes:?}"
        );
        assert!(
            routes.iter().any(|r| r.handler == "health"),
            "got {routes:?}"
        );
    }

    #[test]
    fn detects_express_routes() {
        let src = "\
app.get('/users', listUsers);
router.post('/users', createUser);
";
        let routes = detect_routes("javascript", src);
        assert!(
            routes
                .iter()
                .any(|r| r.method == "get" && r.path == "/users" && r.handler == "listUsers"),
            "got {routes:?}"
        );
        assert!(
            routes
                .iter()
                .any(|r| r.method == "post" && r.handler == "createUser"),
            "got {routes:?}"
        );
    }

    #[test]
    fn non_framework_code_yields_no_routes() {
        let routes = detect_routes("python", "def plain():\n    pass\n");
        assert!(routes.is_empty(), "got {routes:?}");
    }
}
