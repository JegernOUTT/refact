//! Module-scope names a script binds and builtin-named comprehension variables it reads,
//! found from the syntax tree before the script runs.
use ruff_python_ast::{
    self as ast,
    visitor::{self, Visitor},
    ExceptHandler, Expr, ExprContext, Pattern, Stmt,
};
use ruff_text_size::Ranged;

/// Builtin type names Monty resolves without an import (Monty 1.6.0 `Type::from_builtin_name`).
pub const BUILTIN_TYPE_NAMES: &[&str] = &[
    "bool",
    "int",
    "float",
    "complex",
    "str",
    "bytes",
    "list",
    "tuple",
    "dict",
    "set",
    "frozenset",
    "range",
    "slice",
    "iter",
    "type",
    "property",
    "object",
];

/// Whether Monty reads `name` as a builtin when nothing binds it.
pub fn is_builtin(name: &str) -> bool {
    name.parse::<monty_types::BuiltinsFunctions>().is_ok()
        || name.parse::<monty_types::ExcType>().is_ok()
        || BUILTIN_TYPE_NAMES.contains(&name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    Binds,
    Defines,
    Imports,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub name: String,
    pub offset: usize,
    /// Where the binding takes effect: after the value for an assignment, else at `offset`.
    pub effective: usize,
    pub how: How,
    /// The import that binds the name, as `import x` or `from m import x`.
    pub import: Option<String>,
    /// `import os` or `from pathlib import Path`: the import the session itself makes.
    pub canonical: bool,
}

pub struct Scan {
    /// Module-scope bindings in source order.
    pub bindings: Vec<Binding>,
    /// Reads, inside a module-level comprehension, of its own variables named like a builtin.
    pub builtin_reads: Vec<(String, usize)>,
}

/// `None` when the code does not parse.
pub fn scan(code: &str) -> Option<Scan> {
    let parsed = ruff_python_parser::parse_module(code).ok()?;
    let mut walker = Walker::default();
    walker.visit_body(&parsed.syntax().body);
    walker.bindings.sort_by_key(|binding| binding.offset);
    Some(Scan {
        bindings: walker.bindings,
        builtin_reads: walker.reads,
    })
}

#[derive(Default)]
struct Walker<'a> {
    /// Nonzero inside function and class bodies.
    depth: usize,
    /// Variables of the module-level comprehensions being walked.
    comprehensions: Vec<Vec<&'a str>>,
    bindings: Vec<Binding>,
    reads: Vec<(String, usize)>,
}

impl<'a> Walker<'a> {
    fn bind(&mut self, name: &str, offset: usize, how: How) {
        self.bindings.push(Binding {
            name: name.into(),
            offset,
            effective: offset,
            how,
            import: None,
            canonical: false,
        });
    }
    fn assigned(&mut self, targets: &[&'a Expr], value: &Expr) {
        let first = self.bindings.len();
        targets.iter().for_each(|target| self.targets(target));
        for binding in &mut self.bindings[first..] {
            binding.effective = value.end().to_usize();
        }
    }
    fn targets(&mut self, target: &'a Expr) {
        match target {
            Expr::Name(name) => self.bind(name.id.as_str(), name.start().to_usize(), How::Binds),
            Expr::Tuple(tuple) => tuple.elts.iter().for_each(|e| self.targets(e)),
            Expr::List(list) => list.elts.iter().for_each(|e| self.targets(e)),
            Expr::Starred(starred) => self.targets(&starred.value),
            _ => {}
        }
    }
    fn pattern(&mut self, pattern: &'a Pattern) {
        match pattern {
            Pattern::MatchAs(p) => {
                if let Some(name) = &p.name {
                    self.bind(name.as_str(), name.start().to_usize(), How::Binds);
                }
                if let Some(p) = &p.pattern {
                    self.pattern(p);
                }
            }
            Pattern::MatchStar(p) => {
                if let Some(name) = &p.name {
                    self.bind(name.as_str(), name.start().to_usize(), How::Binds);
                }
            }
            Pattern::MatchMapping(p) => {
                if let Some(rest) = &p.rest {
                    self.bind(rest.as_str(), rest.start().to_usize(), How::Binds);
                }
                p.patterns.iter().for_each(|p| self.pattern(p));
            }
            Pattern::MatchSequence(p) => p.patterns.iter().for_each(|p| self.pattern(p)),
            Pattern::MatchOr(p) => p.patterns.iter().for_each(|p| self.pattern(p)),
            Pattern::MatchClass(p) => {
                p.arguments.patterns.iter().for_each(|p| self.pattern(p));
                p.arguments
                    .keywords
                    .iter()
                    .for_each(|k| self.pattern(&k.pattern));
            }
            Pattern::MatchValue(_) | Pattern::MatchSingleton(_) => {}
        }
    }
    fn comprehension(&mut self, generators: &'a [ast::Comprehension], elts: &[&'a Expr]) {
        self.comprehensions.push(Vec::new());
        for generator in generators {
            self.visit_expr(&generator.iter);
            let mut names = Vec::new();
            names_in(&generator.target, &mut names);
            if let Some(scope) = self.comprehensions.last_mut() {
                scope.extend(names);
            }
            for condition in &generator.ifs {
                self.visit_expr(condition);
            }
        }
        for elt in elts {
            self.visit_expr(elt);
        }
        self.comprehensions.pop();
    }
}

fn names_in<'a>(target: &'a Expr, out: &mut Vec<&'a str>) {
    match target {
        Expr::Name(name) => out.push(name.id.as_str()),
        Expr::Tuple(tuple) => tuple.elts.iter().for_each(|e| names_in(e, out)),
        Expr::List(list) => list.elts.iter().for_each(|e| names_in(e, out)),
        Expr::Starred(starred) => names_in(&starred.value, out),
        _ => {}
    }
}

impl<'a> Visitor<'a> for Walker<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if self.depth > 0 {
            if let Stmt::Global(global) = stmt {
                for name in &global.names {
                    self.bind(name.as_str(), name.start().to_usize(), How::Binds);
                }
            }
            return visitor::walk_stmt(self, stmt);
        }
        match stmt {
            Stmt::FunctionDef(f) => {
                for decorator in &f.decorator_list {
                    self.visit_decorator(decorator);
                }
                self.visit_parameters(&f.parameters);
                if let Some(returns) = &f.returns {
                    self.visit_annotation(returns);
                }
                self.bind(f.name.as_str(), f.name.start().to_usize(), How::Defines);
                self.depth += 1;
                self.visit_body(&f.body);
                self.depth -= 1;
            }
            Stmt::ClassDef(c) => {
                for decorator in &c.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(arguments) = &c.arguments {
                    self.visit_arguments(arguments);
                }
                self.bind(c.name.as_str(), c.name.start().to_usize(), How::Defines);
                self.depth += 1;
                self.visit_body(&c.body);
                self.depth -= 1;
            }
            Stmt::Import(import) => {
                for alias in &import.names {
                    let module = alias.name.as_str();
                    let (name, offset) = match &alias.asname {
                        Some(asname) => (asname.as_str(), asname.start()),
                        None => (
                            module.split('.').next().unwrap_or(module),
                            alias.name.start(),
                        ),
                    };
                    let canonical = matches!(module, "os" | "os.path" | "re" | "json" | "asyncio")
                        && alias
                            .asname
                            .as_ref()
                            .is_none_or(|asname| asname.as_str() == module);
                    self.bindings.push(Binding {
                        name: name.into(),
                        offset: offset.to_usize(),
                        effective: offset.to_usize(),
                        how: How::Imports,
                        import: Some(format!("import {module}")),
                        canonical,
                    });
                }
            }
            Stmt::ImportFrom(import) => {
                let module = format!(
                    "{}{}",
                    ".".repeat(import.level as usize),
                    import.module.as_ref().map_or("", |m| m.as_str())
                );
                for alias in &import.names {
                    if alias.name.as_str() == "*" {
                        continue;
                    }
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                    self.bindings.push(Binding {
                        name: bound.as_str().into(),
                        offset: bound.start().to_usize(),
                        effective: bound.start().to_usize(),
                        how: How::Imports,
                        import: Some(format!("from {module} import {}", alias.name.as_str())),
                        canonical: module == "pathlib"
                            && alias.name.as_str() == "Path"
                            && bound.as_str() == "Path",
                    });
                }
            }
            Stmt::Assign(assign) => {
                let targets: Vec<&Expr> = assign.targets.iter().collect();
                self.assigned(&targets, &assign.value);
                visitor::walk_stmt(self, stmt);
            }
            Stmt::AugAssign(assign) => {
                self.assigned(&[&*assign.target], &assign.value);
                visitor::walk_stmt(self, stmt);
            }
            Stmt::AnnAssign(assign) => {
                if let Some(value) = &assign.value {
                    self.assigned(&[&*assign.target], value);
                }
                visitor::walk_stmt(self, stmt);
            }
            Stmt::TypeAlias(alias) => {
                self.targets(&alias.name);
                visitor::walk_stmt(self, stmt);
            }
            Stmt::For(for_) => {
                self.targets(&for_.target);
                visitor::walk_stmt(self, stmt);
            }
            Stmt::With(with) => {
                for item in &with.items {
                    if let Some(vars) = &item.optional_vars {
                        self.targets(vars);
                    }
                }
                visitor::walk_stmt(self, stmt);
            }
            Stmt::Try(try_) => {
                for handler in &try_.handlers {
                    let ExceptHandler::ExceptHandler(handler) = handler;
                    if let Some(name) = &handler.name {
                        self.bind(name.as_str(), name.start().to_usize(), How::Binds);
                    }
                }
                visitor::walk_stmt(self, stmt);
            }
            Stmt::Match(match_) => {
                for case in &match_.cases {
                    self.pattern(&case.pattern);
                }
                visitor::walk_stmt(self, stmt);
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }
    fn visit_expr(&mut self, expr: &'a Expr) {
        if self.depth > 0 {
            return;
        }
        match expr {
            Expr::Named(named) => {
                self.assigned(&[&*named.target], &named.value);
                self.visit_expr(&named.value);
            }
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &lambda.parameters {
                    self.visit_parameters(parameters);
                }
            }
            Expr::ListComp(c) => self.comprehension(&c.generators, &[&*c.elt]),
            Expr::SetComp(c) => self.comprehension(&c.generators, &[&*c.elt]),
            Expr::Generator(c) => self.comprehension(&c.generators, &[&*c.elt]),
            Expr::DictComp(c) => {
                let mut elts: Vec<&'a Expr> = c.key.as_deref().into_iter().collect();
                elts.push(&c.value);
                self.comprehension(&c.generators, &elts);
            }
            Expr::Name(name) if name.ctx == ExprContext::Load => {
                let id = name.id.as_str();
                if is_builtin(id) && self.comprehensions.iter().any(|scope| scope.contains(&id)) {
                    self.reads.push((id.into(), name.start().to_usize()));
                }
            }
            _ => visitor::walk_expr(self, expr),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{client::*, kernel::FakeOs, protocol::*};

    #[tokio::test]
    async fn every_listed_builtin_type_name_is_a_builtin_of_this_interpreter() {
        let mut c =
            KernelClient::spawn_in_thread(Box::new(FakeOs::default()), ClientOptions::default())
                .unwrap();
        c.next_event().await;
        c.send(KernelCommand::Feed {
            run_id: "t".into(),
            code: format!(
                "[repr(x) for x in [{}]]",
                super::BUILTIN_TYPE_NAMES.join(", ")
            ),
            globals: vec![],
            limits: RunLimits::default(),
        })
        .await
        .unwrap();
        let ClientEvent::Kernel(KernelEvent::Completed {
            value, ..
        }) = c.next_event().await
        else {
            panic!("the builtin names did not evaluate")
        };
        for (name, repr) in super::BUILTIN_TYPE_NAMES
            .iter()
            .zip(value.as_array().unwrap())
        {
            let repr = repr.as_str().unwrap();
            assert!(
                repr.starts_with("<class '") || repr == format!("<built-in function {name}>"),
                "{name}: {repr}"
            );
        }
    }
}
