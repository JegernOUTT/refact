use tree_sitter::Node;

use crate::frameworks::DetectedRoute;

const HTTP_METHODS: &[&str] = &[
    "get", "post", "put", "delete", "patch", "head", "options", "route", "all", "use", "handle",
];

const HTTP_METHOD_NAMES: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

/// Detect pragmatic framework route -> handler associations for the given language.
///
/// This module intentionally keeps the detectors heuristic and self-contained. It always parses
/// the source first, then dispatches to language/framework-specific detectors.
pub fn detect_framework_routes(lang: &str, text: &str) -> Vec<DetectedRoute> {
    let Some(tree) = crate::parse_tree(lang, text) else {
        return Vec::new();
    };

    let bytes = text.as_bytes();
    let mut out = Vec::new();
    match lang {
        "python" => detect_python(tree.root_node(), bytes, &mut out),
        "javascript" | "jsx" | "typescript" | "tsx" => {
            detect_js_ts(tree.root_node(), bytes, text, &mut out)
        }
        "go" => detect_go(tree.root_node(), bytes, &mut out),
        "rust" => detect_rust(tree.root_node(), bytes, text, &mut out),
        "java" | "kotlin" => detect_spring(text, &mut out),
        "php" => detect_laravel(text, &mut out),
        "ruby" => detect_rails(text, &mut out),
        _ => {}
    }
    out
}

fn node_text<'a>(node: Node, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('r').unwrap_or(s);
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

fn first_quoted(text: &str) -> Option<String> {
    let mut chars = text.char_indices();
    while let Some((start, ch)) = chars.next() {
        if ch == '"' || ch == '\'' || ch == '`' {
            let rest = &text[start + ch.len_utf8()..];
            let mut escaped = false;
            for (offset, c) in rest.char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if c == '\\' {
                    escaped = true;
                    continue;
                }
                if c == ch {
                    return Some(rest[..offset].to_string());
                }
            }
        }
    }
    None
}

fn split_top_level_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in args.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(args[start..idx].trim().to_string());
                start = idx + 1;
            }
            _ => {}
        }
    }
    if start <= args.len() {
        let tail = args[start..].trim();
        if !tail.is_empty() {
            out.push(tail.to_string());
        }
    }
    out
}

fn call_args_text(call_text: &str) -> Option<&str> {
    let open = call_text.find('(')?;
    let close = call_text.rfind(')')?;
    (close > open).then_some(&call_text[open + 1..close])
}

fn ident_tail(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty()
        || text.contains("=>")
        || text.starts_with('|')
        || text.starts_with('"')
        || text.starts_with('\'')
        || text.starts_with('`')
    {
        return None;
    }
    if let Some(as_view) = text.find(".as_view") {
        return ident_tail(&text[..as_view]);
    }
    if let Some(hash) = text.rfind('#') {
        return Some(strip_quotes(&text[hash + 1..]));
    }
    if let Some(at) = text.rfind('@') {
        return Some(strip_quotes(&text[at + 1..]));
    }
    let cleaned = text
        .trim_end_matches(';')
        .trim_end_matches(')')
        .trim_end_matches(']')
        .trim();
    let mut last = String::new();
    for part in cleaned.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$')) {
        if !part.is_empty() && part != "class" && part != "web" && part != "Route" {
            last = part.to_string();
        }
    }
    (!last.is_empty()).then_some(last)
}

fn first_string_arg_from_call(call_text: &str) -> Option<String> {
    let args = call_args_text(call_text)?;
    split_top_level_args(args)
        .into_iter()
        .find_map(|arg| first_quoted(&arg))
}

fn detect_python(node: Node, bytes: &[u8], out: &mut Vec<DetectedRoute>) {
    if node.kind() == "call" {
        if let Some(route) = django_route_from_call(node_text(node, bytes)) {
            out.push(route);
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        detect_python(child, bytes, out);
    }
}

fn django_route_from_call(text: &str) -> Option<DetectedRoute> {
    let name = text.split('(').next()?.trim();
    let func = name.rsplit('.').next().unwrap_or(name);
    if !matches!(func, "path" | "re_path" | "url") {
        return None;
    }
    let args = split_top_level_args(call_args_text(text)?);
    let path = args.first().and_then(|a| first_quoted(a))?;
    let handler_arg = args.get(1)?;
    let handler = ident_tail(handler_arg)?;
    Some(DetectedRoute {
        method: "ANY".to_string(),
        path,
        handler,
    })
}

fn detect_js_ts(node: Node, bytes: &[u8], source: &str, out: &mut Vec<DetectedRoute>) {
    detect_member_calls(node, bytes, MemberStyle::Lowercase, out);
    detect_nestjs(source, out);
    detect_nextjs(source, out);
}

#[derive(Clone, Copy)]
enum MemberStyle {
    Lowercase,
    Titlecase,
}

fn detect_member_calls(node: Node, bytes: &[u8], style: MemberStyle, out: &mut Vec<DetectedRoute>) {
    if node.kind().contains("call") {
        if let Some(route) = member_route_from_text(node_text(node, bytes), style) {
            out.push(route);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        detect_member_calls(child, bytes, style, out);
    }
}

fn member_route_from_text(text: &str, style: MemberStyle) -> Option<DetectedRoute> {
    let open = text.find('(')?;
    let before = text[..open].trim();
    if !before.contains('.') && !before.contains("::") {
        return None;
    }
    let raw_method = before.rsplit(|c| c == '.' || c == ':').next()?.trim();
    let lower = raw_method.to_ascii_lowercase();
    if !HTTP_METHODS.contains(&lower.as_str()) {
        return None;
    }
    let path = first_string_arg_from_call(text)?;
    let args = split_top_level_args(call_args_text(text)?);
    let handler = args.iter().skip(1).rev().find_map(|arg| ident_tail(arg))?;
    let method = match style {
        MemberStyle::Lowercase => lower,
        MemberStyle::Titlecase => {
            if lower == "handle" {
                "ANY".to_string()
            } else {
                lower.to_ascii_uppercase()
            }
        }
    };
    Some(DetectedRoute {
        method,
        path,
        handler,
    })
}

fn detect_nestjs(source: &str, out: &mut Vec<DetectedRoute>) {
    let mut pending: Option<(String, String)> = None;
    for raw in source.lines() {
        let line = raw.trim();
        if let Some((method, path)) = nest_decorator(line) {
            pending = Some((method, path));
            continue;
        }
        if let Some((method, path)) = pending.take() {
            if let Some(handler) = method_name_from_signature(line) {
                out.push(DetectedRoute {
                    method,
                    path,
                    handler,
                });
            } else {
                pending = Some((method, path));
            }
        }
    }
}

fn nest_decorator(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix('@')?;
    let name_end = rest.find('(').unwrap_or(rest.len());
    let name = &rest[..name_end];
    let method = match name {
        "Get" => "GET",
        "Post" => "POST",
        "Put" => "PUT",
        "Delete" => "DELETE",
        "Patch" => "PATCH",
        _ => return None,
    };
    Some((method.to_string(), first_quoted(line).unwrap_or_default()))
}

fn detect_nextjs(source: &str, out: &mut Vec<DetectedRoute>) {
    for line in source.lines() {
        let line = line.trim();
        if !line.starts_with("export ") || !line.contains("function ") {
            continue;
        }
        if let Some(name) = line
            .split("function ")
            .nth(1)
            .and_then(|s| s.split('(').next())
        {
            let name = name.trim();
            if HTTP_METHOD_NAMES.contains(&name) {
                out.push(DetectedRoute {
                    method: name.to_string(),
                    path: String::new(),
                    handler: name.to_string(),
                });
            }
        }
    }
}

fn method_name_from_signature(line: &str) -> Option<String> {
    if line.starts_with('@') || line.starts_with("//") || line.is_empty() {
        return None;
    }
    let before = line.split('(').next()?.trim();
    let name = before
        .split_whitespace()
        .last()?
        .trim_start_matches("fun ")
        .trim();
    let name = name.rsplit('.').next().unwrap_or(name);
    if name.is_empty() || name == "if" || name == "for" || name == "while" {
        None
    } else {
        Some(name.to_string())
    }
}

fn detect_go(node: Node, bytes: &[u8], out: &mut Vec<DetectedRoute>) {
    detect_member_calls(node, bytes, MemberStyle::Titlecase, out);
}

fn detect_rust(node: Node, bytes: &[u8], source: &str, out: &mut Vec<DetectedRoute>) {
    detect_rust_calls(node, bytes, out);
    detect_actix_attrs(source, out);
}

fn detect_rust_calls(node: Node, bytes: &[u8], out: &mut Vec<DetectedRoute>) {
    if node.kind().contains("call") {
        let text = node_text(node, bytes);
        if let Some(route) = axum_route_from_text(text).or_else(|| actix_route_from_text(text)) {
            out.push(route);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        detect_rust_calls(child, bytes, out);
    }
}

fn axum_route_from_text(text: &str) -> Option<DetectedRoute> {
    if !text.contains(".route") && !text.trim_start().starts_with("route") {
        return None;
    }
    let path = first_string_arg_from_call(text)?;
    for method in ["get", "post", "put", "delete", "patch"] {
        let needle = format!("{method}(");
        if let Some(pos) = text.find(&needle) {
            let rest = &text[pos + needle.len()..];
            let handler = ident_tail(rest.split(')').next().unwrap_or(rest))?;
            return Some(DetectedRoute {
                method: method.to_ascii_uppercase(),
                path,
                handler,
            });
        }
    }
    None
}

fn actix_route_from_text(text: &str) -> Option<DetectedRoute> {
    if !text.contains(".route") || !text.contains(".to(") {
        return None;
    }
    let path = first_string_arg_from_call(text)?;
    let method = ["get", "post", "put", "delete", "patch"]
        .into_iter()
        .find(|m| text.contains(&format!("web::{m}")))?;
    let to_pos = text.rfind(".to(")?;
    let handler = ident_tail(text[to_pos + 4..].split(')').next().unwrap_or(""))?;
    Some(DetectedRoute {
        method: method.to_ascii_uppercase(),
        path,
        handler,
    })
}

fn detect_actix_attrs(source: &str, out: &mut Vec<DetectedRoute>) {
    let mut pending: Option<(String, String)> = None;
    for raw in source.lines() {
        let line = raw.trim();
        if let Some(attr) = line.strip_prefix("#[").and_then(|s| s.strip_suffix(']')) {
            let name = attr.split('(').next().unwrap_or(attr);
            if ["get", "post", "put", "delete", "patch"].contains(&name) {
                pending = Some((
                    name.to_ascii_uppercase(),
                    first_quoted(attr).unwrap_or_default(),
                ));
                continue;
            }
        }
        if let Some((method, path)) = pending.take() {
            if let Some(handler) = line
                .strip_prefix("async fn ")
                .or_else(|| line.strip_prefix("fn "))
                .and_then(|s| s.split('(').next())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                out.push(DetectedRoute {
                    method,
                    path,
                    handler: handler.to_string(),
                });
            } else {
                pending = Some((method, path));
            }
        }
    }
}

fn detect_spring(source: &str, out: &mut Vec<DetectedRoute>) {
    let mut pending: Option<(String, String)> = None;
    for raw in source.lines() {
        let line = raw.trim();
        if let Some(route) = spring_annotation(line) {
            pending = Some(route);
            continue;
        }
        if let Some((method, path)) = pending.take() {
            if let Some(handler) = method_name_from_signature(line) {
                out.push(DetectedRoute {
                    method,
                    path,
                    handler,
                });
            } else {
                pending = Some((method, path));
            }
        }
    }
}

fn spring_annotation(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix('@')?;
    let name = rest.split('(').next().unwrap_or(rest).trim();
    let method = match name {
        "GetMapping" => "GET".to_string(),
        "PostMapping" => "POST".to_string(),
        "PutMapping" => "PUT".to_string(),
        "DeleteMapping" => "DELETE".to_string(),
        "PatchMapping" => "PATCH".to_string(),
        "RequestMapping" => request_mapping_method(line).unwrap_or_else(|| "ANY".to_string()),
        _ => return None,
    };
    Some((method, first_quoted(line).unwrap_or_default()))
}

fn request_mapping_method(line: &str) -> Option<String> {
    for method in HTTP_METHOD_NAMES {
        if line.contains(method) {
            return Some((*method).to_string());
        }
    }
    None
}

fn detect_laravel(source: &str, out: &mut Vec<DetectedRoute>) {
    for line in source.lines() {
        let line = line.trim();
        let Some(pos) = line.find("Route::") else {
            continue;
        };
        let rest = &line[pos + "Route::".len()..];
        let method = rest
            .split('(')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !HTTP_METHODS.contains(&method.as_str()) {
            continue;
        }
        let Some(args) = call_args_text(rest) else {
            continue;
        };
        let parts = split_top_level_args(args);
        let Some(path) = parts.first().and_then(|p| first_quoted(p)) else {
            continue;
        };
        let Some(handler_arg) = parts.get(1) else {
            continue;
        };
        let handler = if handler_arg.contains("::class") {
            let quoted: Vec<String> = handler_arg.split(',').filter_map(first_quoted).collect();
            quoted.last().cloned()
        } else {
            first_quoted(handler_arg).and_then(|s| ident_tail(&s))
        };
        if let Some(handler) = handler {
            out.push(DetectedRoute {
                method,
                path,
                handler,
            });
        }
    }
}

fn detect_rails(source: &str, out: &mut Vec<DetectedRoute>) {
    for line in source.lines() {
        let line = line.trim();
        let method = line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if !HTTP_METHODS.contains(&method.as_str()) {
            continue;
        }
        let Some(path) = first_quoted(line) else {
            continue;
        };
        let Some(to_pos) = line.find("to:") else {
            continue;
        };
        let Some(target) = first_quoted(&line[to_pos..]) else {
            continue;
        };
        let Some(handler) = ident_tail(&target) else {
            continue;
        };
        out.push(DetectedRoute {
            method,
            path,
            handler,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(routes: &[DetectedRoute], method: &str, handler: &str) -> bool {
        routes
            .iter()
            .any(|r| r.method == method && r.handler == handler)
    }

    #[test]
    fn detects_django_urls() {
        let src = r#"
from django.urls import path
urlpatterns = [
    path("users/", views.list_users),
    path("items/", ItemView.as_view()),
]
"#;
        let routes = detect_framework_routes("python", src);
        assert!(has(&routes, "ANY", "list_users"), "got {routes:?}");
        assert!(has(&routes, "ANY", "ItemView"), "got {routes:?}");
    }

    #[test]
    fn detects_nestjs_decorators() {
        let src = r#"
@Controller('users')
export class UsersController {
  @Get('/active')
  findActive() { return []; }
}
"#;
        let routes = detect_framework_routes("typescript", src);
        assert!(has(&routes, "GET", "findActive"), "got {routes:?}");
    }

    #[test]
    fn detects_nextjs_app_router_exports() {
        let src = r#"
export async function GET(request: Request) { return Response.json({}); }
"#;
        let routes = detect_framework_routes("typescript", src);
        assert!(has(&routes, "GET", "GET"), "got {routes:?}");
    }

    #[test]
    fn detects_hono_koa_fastify_shape() {
        let src = "app.get('/ping', pingHandler);";
        let routes = detect_framework_routes("javascript", src);
        assert!(has(&routes, "get", "pingHandler"), "got {routes:?}");
    }

    #[test]
    fn detects_gin_echo_chi_member_routes() {
        let src = r#"
package main
func main() {
    r.GET("/users", listUsers)
}
"#;
        let routes = detect_framework_routes("go", src);
        assert!(has(&routes, "GET", "listUsers"), "got {routes:?}");
    }

    #[test]
    fn detects_axum_and_actix_routes() {
        let src = r#"
use axum::{routing::get, Router};
#[get("/health")]
async fn health() {}
fn app() -> Router {
    Router::new().route("/users", get(list_users))
}
"#;
        let routes = detect_framework_routes("rust", src);
        assert!(has(&routes, "GET", "list_users"), "got {routes:?}");
        assert!(has(&routes, "GET", "health"), "got {routes:?}");
    }

    #[test]
    fn detects_spring_mappings() {
        let src = r#"
class UsersController {
  @GetMapping("/users")
  public List<User> listUsers() { return List.of(); }
}
"#;
        let routes = detect_framework_routes("java", src);
        assert!(has(&routes, "GET", "listUsers"), "got {routes:?}");
    }

    #[test]
    fn detects_laravel_routes() {
        let src = r#"<?php
Route::get('/users', [UserController::class, 'index']);
Route::post('/users', 'UserController@store');
"#;
        let routes = detect_framework_routes("php", src);
        assert!(has(&routes, "get", "index"), "got {routes:?}");
        assert!(has(&routes, "post", "store"), "got {routes:?}");
    }

    #[test]
    fn detects_rails_routes() {
        let src = "Rails.application.routes.draw do\n  get '/users', to: 'users#index'\nend";
        let routes = detect_framework_routes("ruby", src);
        assert!(has(&routes, "get", "index"), "got {routes:?}");
    }

    #[test]
    fn plain_code_yields_no_routes() {
        let routes = detect_framework_routes("python", "def plain():\n    return 1\n");
        assert!(routes.is_empty(), "got {routes:?}");
    }
}
