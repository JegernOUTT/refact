use crate::protocol::{Dimension, Exceeded};
use monty_types::{
    MontyObject, ObjectRef,
    unstable::{self, MontyNode},
};
use serde_json::{json, Value};
use std::fmt::{self, Write};
pub const MAX_DEPTH: usize = 32;
pub const MAX_NODES: usize = 4096;
pub const MAX_REPR_BYTES: usize = 4096;
pub const MAX_VALUE_BYTES: u64 = 32 * 1024 * 1024;

/// Exact JSON-encoded size of a value as serde_json writes it, its node count and nesting depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JsonSize {
    pub nodes: usize,
    pub bytes: usize,
    pub depth: usize,
}
pub fn json_size(v: &Value) -> JsonSize {
    fn walk(v: &Value, depth: usize, s: &mut JsonSize) {
        s.nodes += 1;
        s.depth = s.depth.max(depth);
        let own = match v {
            Value::Null => 4,
            Value::Bool(b) => 4 + usize::from(!*b),
            Value::Number(n) => {
                let mut c = Counter(0);
                let _ = write!(c, "{n}");
                c.0
            }
            Value::String(s) => json_string_bytes(s),
            Value::Array(a) => {
                for v in a {
                    walk(v, depth + 1, s);
                }
                2 + a.len().saturating_sub(1)
            }
            Value::Object(o) => {
                for (k, v) in o {
                    s.bytes = s.bytes.saturating_add(json_string_bytes(k) + 1);
                    walk(v, depth + 1, s);
                }
                2 + o.len().saturating_sub(1)
            }
        };
        s.bytes = s.bytes.saturating_add(own);
    }
    let mut s = JsonSize::default();
    walk(v, 0, &mut s);
    s
}
struct Counter(usize);
impl Write for Counter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 += s.len();
        Ok(())
    }
}
/// Node budget of a result byte budget: one value per two bytes (plus one). The smallest
/// value in a container (`0,`) takes two bytes, so every result within the byte budget fits.
pub fn result_nodes_for(max_bytes: u64) -> u32 {
    (result_byte_limit(max_bytes) / 2 + 1).min(u32::MAX as u64) as u32
}
/// Effective byte limit of a host result after the kernel's clamp.
pub fn result_byte_limit(max_bytes: u64) -> u64 {
    max_bytes.min(MAX_VALUE_BYTES)
}
pub fn fits_result(size: &JsonSize, max_nodes: u32, max_bytes: u64) -> bool {
    size.depth <= MAX_DEPTH
        && size.nodes <= max_nodes as usize
        && size.bytes as u64 <= result_byte_limit(max_bytes)
}
/// `what` names the value, e.g. "f() returned a result".
pub fn too_large_message(what: &str, size: &JsonSize, max_nodes: u32, max_bytes: u64) -> String {
    let depth = if size.depth > MAX_DEPTH {
        format!(", nested {} levels deep (limit {MAX_DEPTH})", size.depth)
    } else {
        String::new()
    };
    format!(
        "{what} of {} bytes and {} values{depth}, over the limit a script can receive at once ({} bytes, {max_nodes} values); narrow the call (filter, page or limit what it returns)",
        size.bytes,
        size.nodes,
        result_byte_limit(max_bytes),
    )
}
/// Converts a host result for a script; never cuts it. `Err` carries the measured size.
pub fn from_json_limited(
    v: &Value,
    max_nodes: u32,
    max_bytes: u64,
) -> Result<MontyObject, JsonSize> {
    let size = json_size(v);
    if !fits_result(&size, max_nodes, max_bytes) {
        return Err(size);
    }
    Ok(from_json_inner(v))
}
fn from_json_inner(v: &Value) -> MontyObject {
    match v {
        Value::Null => MontyObject::none(),
        Value::Bool(b) => MontyObject::bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                MontyObject::int(i)
            } else if let Some(i) = n.as_u64() {
                MontyObject::bigint(i.into())
            } else {
                MontyObject::float(n.as_f64().unwrap())
            }
        }
        Value::String(s) => MontyObject::string(s),
        Value::Array(a) => MontyObject::list(a.iter().map(from_json_inner)),
        Value::Object(o) => MontyObject::dict(
            o.iter()
                .map(|(k, v)| (MontyObject::string(k), from_json_inner(v))),
        ),
    }
}
/// How a value JSON cannot hold is converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Results and `show()`: such a value becomes its bounded repr.
    Display,
    /// Sync helper arguments: a path becomes its string, any other such value is refused.
    Helper,
    /// Async call arguments: such a value is refused.
    Strict,
}
pub struct Budget {
    nodes: usize,
    bytes: usize,
    max_nodes: usize,
    max_bytes: usize,
    mode: Mode,
    /// A limit cut the value; `exceeded` names the first one.
    pub truncated: bool,
    pub exceeded: Option<Exceeded>,
}
/// A call argument JSON cannot carry.
#[derive(Debug)]
pub struct Unsupported {
    /// Innermost segment first; the argument name is last.
    path: Vec<String>,
    detail: String,
}
impl Unsupported {
    pub fn message(&self, function: &str) -> String {
        let path: String = self.path.iter().rev().map(String::as_str).collect();
        format!("{function}() argument {path} {}", self.detail)
    }
}
enum Stop {
    Limit,
    Unsupported(Unsupported),
}
impl Stop {
    fn at(self, segment: impl FnOnce() -> String) -> Self {
        match self {
            Stop::Unsupported(mut unsupported) => {
                unsupported.path.push(segment());
                Stop::Unsupported(unsupported)
            }
            limit => limit,
        }
    }
}
impl Budget {
    pub fn new(max_nodes: u32, max_bytes: u64) -> Self {
        Self::with_mode(max_nodes, max_bytes, Mode::Display)
    }
    pub fn with_mode(max_nodes: u32, max_bytes: u64, mode: Mode) -> Self {
        Self {
            nodes: 0,
            bytes: 0,
            max_nodes: max_nodes as usize,
            max_bytes: max_bytes.min(MAX_VALUE_BYTES) as usize,
            mode,
            truncated: false,
            exceeded: None,
        }
    }
    fn exceed(&mut self, dimension: Dimension, at_least: usize, limit: usize) {
        self.truncated = true;
        self.exceeded.get_or_insert(Exceeded {
            dimension,
            at_least: at_least as u64,
            limit: limit as u64,
        });
    }
    /// Adds `nodes` values and `bytes` of the JSON text sent; `false` once a limit is passed.
    fn charge(&mut self, nodes: usize, bytes: usize) -> bool {
        self.nodes += nodes;
        self.bytes = self.bytes.saturating_add(bytes);
        if self.nodes > self.max_nodes {
            self.exceed(Dimension::Values, self.nodes, self.max_nodes);
        } else if self.bytes > self.max_bytes {
            self.exceed(Dimension::Bytes, self.bytes, self.max_bytes);
        } else {
            return true;
        }
        false
    }
    /// Charges the JSON text around top-level arguments: the brackets, the commas between
    /// `items`, and each keyword name with its quotes and colon.
    pub fn frame<'a>(&mut self, items: usize, keys: impl Iterator<Item = &'a str>) {
        let keys = keys.fold(0usize, |n, key| {
            n.saturating_add(json_string_bytes(key) + 1)
        });
        self.charge(0, keys.saturating_add(2 + items.saturating_sub(1)));
    }
    /// Converts a result or `show()` value; a limit marks `truncated` and keeps a repr.
    pub fn convert(&mut self, v: ObjectRef<'_>) -> Value {
        self.argument("value", v)
            .unwrap_or_else(|_| json!({"__repr__":bounded_repr(v, MAX_REPR_BYTES).0}))
    }
    /// Converts the argument `name`; a value JSON cannot carry is refused unless in display mode.
    pub fn argument(&mut self, name: &str, v: ObjectRef<'_>) -> Result<Value, Unsupported> {
        if self.exceeded.is_some() {
            return Ok(json!({"__repr__":"<truncated>"}));
        }
        match self.inner(v, 0) {
            Ok(value) => Ok(value),
            Err(Stop::Limit) if self.mode == Mode::Display => {
                Ok(json!({"__repr__":bounded_repr(v, MAX_REPR_BYTES).0}))
            }
            Err(Stop::Limit) => Ok(json!({"__repr__":"<truncated>"})),
            Err(Stop::Unsupported(mut unsupported)) => {
                unsupported.path.push(label(name));
                Err(unsupported)
            }
        }
    }
    fn fallback(&mut self, v: ObjectRef<'_>) -> Value {
        let room = MAX_REPR_BYTES.min(self.max_bytes.saturating_sub(self.bytes) / 6);
        let (repr, cut) = bounded_repr(v, room);
        if cut {
            let shown = repr.chars().count();
            self.exceed(Dimension::Text, shown + 1, shown);
        }
        self.nodes += 1;
        self.bytes = self
            .bytes
            .saturating_add(REPR_WRAPPER_BYTES + json_string_bytes(&repr));
        json!({"__repr__":repr})
    }
    fn other(&mut self, v: ObjectRef<'_>) -> Result<Value, Stop> {
        if self.mode == Mode::Display {
            return Ok(self.fallback(v));
        }
        let allowed = if self.mode == Mode::Helper {
            "str, int, float, bool, None, list, dict or Path"
        } else {
            "str, int, float, bool, None, list or dict"
        };
        let node = unstable::node(v);
        let detail = match (v.type_name(), node) {
            ("int", _) => "is an int too large for JSON; pass it as a string.".into(),
            ("float", _) => "is a float JSON cannot represent (infinity or NaN).".into(),
            (_, MontyNode::Repr(text)) => format!("must be {allowed}, not {}.", repr_type(text)),
            (type_name, node) => {
                let builtin = match node {
                    MontyNode::BuiltinFunction(f) => Some(f.to_string()),
                    MontyNode::Type(t) => Some(t.to_string()),
                    _ => None,
                };
                let hint = match (builtin, node) {
                    (Some(name), _) => format!(" At module level a comprehension variable named like a builtin (here '{name}') reads the builtin in this interpreter; rename the variable."),
                    (_, MontyNode::Path(_)) => " Pass str(path).".into(),
                    (_, MontyNode::Bytes(_)) => " Decode it first, e.g. data.decode().".into(),
                    (_, MontyNode::Set(_) | MontyNode::FrozenSet(_)) => {
                        " Pass a list, e.g. sorted(values).".into()
                    }
                    _ => String::new(),
                };
                format!("must be {allowed}, not {type_name}.{hint}")
            }
        };
        Err(Stop::Unsupported(Unsupported {
            path: vec![],
            detail,
        }))
    }
    fn leaf(&mut self, value: Value) -> Result<Value, Stop> {
        let mut size = Counter(0);
        let _ = write!(size, "{value}");
        if self.charge(1, size.0) {
            Ok(value)
        } else {
            Err(Stop::Limit)
        }
    }
    fn inner(&mut self, v: ObjectRef<'_>, depth: usize) -> Result<Value, Stop> {
        if depth > MAX_DEPTH {
            self.nodes += 1;
            self.exceed(Dimension::Depth, depth, MAX_DEPTH);
            return Err(Stop::Limit);
        }
        Ok(match v.type_name() {
            "NoneType" => self.leaf(Value::Null)?,
            "bool" => self.leaf(json!(v.as_bool().unwrap()))?,
            "int" => match v.as_int() {
                Some(i) => self.leaf(json!(i))?,
                None => self.other(v)?,
            },
            "float" => match v.as_float().filter(|f| f.is_finite()) {
                Some(f) => self.leaf(json!(f))?,
                None => self.other(v)?,
            },
            "str" => {
                let s = v.as_str().unwrap();
                let at_least = self.bytes.saturating_add(s.len() + 2);
                if at_least > self.max_bytes {
                    self.nodes += 1;
                    self.exceed(Dimension::Bytes, at_least, self.max_bytes);
                    return Err(Stop::Limit);
                }
                if !self.charge(1, json_string_bytes(s)) {
                    return Err(Stop::Limit);
                }
                json!(s)
            }
            "list" | "tuple" | "set" | "frozenset"
                if self.mode == Mode::Display || matches!(v.type_name(), "list" | "tuple") =>
            {
                let (MontyNode::List(ids)
                | MontyNode::Tuple(ids)
                | MontyNode::Set(ids)
                | MontyNode::FrozenSet(ids)) = unstable::node(v)
                else {
                    unreachable!()
                };
                if !self.charge(1, 1 + ids.len().max(1)) {
                    return Err(Stop::Limit);
                }
                if self.nodes.saturating_add(ids.len()) > self.max_nodes {
                    self.exceed(
                        Dimension::Values,
                        self.nodes.saturating_add(ids.len()),
                        self.max_nodes,
                    );
                    return Err(Stop::Limit);
                }
                let mut out = Vec::new();
                for (i, id) in ids.iter().enumerate() {
                    out.push(
                        self.inner(unstable::child(v, *id), depth + 1)
                            .map_err(|stop| stop.at(|| format!("[{i}]")))?,
                    );
                }
                Value::Array(out)
            }
            "dict" => {
                let MontyNode::Dict(pairs) = unstable::node(v) else {
                    unreachable!()
                };
                if !self.charge(1, 1 + pairs.len().max(1)) {
                    return Err(Stop::Limit);
                }
                if self.nodes.saturating_add(pairs.len()) > self.max_nodes {
                    self.exceed(
                        Dimension::Values,
                        self.nodes.saturating_add(pairs.len()),
                        self.max_nodes,
                    );
                    return Err(Stop::Limit);
                }
                let mut out = serde_json::Map::new();
                for (k, id) in pairs {
                    let key = unstable::child(v, *k);
                    let Some(k) = key.as_str() else {
                        if self.mode == Mode::Display {
                            return Ok(self.fallback(v));
                        }
                        return Err(Stop::Unsupported(Unsupported {
                            path: vec![],
                            detail: format!(
                                "has a dict key of type {}; dict keys must be str.",
                                key.type_name()
                            ),
                        }));
                    };
                    if !self.charge(0, json_string_bytes(k) + 1) {
                        return Err(Stop::Limit);
                    }
                    let value = self
                        .inner(unstable::child(v, *id), depth + 1)
                        .map_err(|stop| stop.at(|| format!("['{}']", label(k))))?;
                    out.insert(k.into(), value);
                }
                Value::Object(out)
            }
            "PosixPath" if self.mode == Mode::Helper => {
                let MontyNode::Path(path) = unstable::node(v) else {
                    unreachable!()
                };
                if !self.charge(1, json_string_bytes(path)) {
                    return Err(Stop::Limit);
                }
                json!(path)
            }
            _ => self.other(v)?,
        })
    }
}
/// A key or argument name as an argument path shows it: escaped, at most 40 characters.
fn label(name: &str) -> String {
    let mut out: String = name.chars().take(40).flat_map(char::escape_debug).collect();
    if name.chars().nth(40).is_some() {
        out.push('…');
    }
    out
}
/// The type an opaque repr stands for: its first word, as in `<function f at 0x…>`.
fn repr_type(text: &str) -> &str {
    if text.starts_with("<built-in ") {
        return "builtin_function_or_method";
    }
    let word = text
        .strip_prefix('<')
        .map(|rest| {
            rest.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
                .next()
                .unwrap_or_default()
        })
        .unwrap_or_default();
    match word {
        "" => "object",
        "bound" => "method",
        word => word,
    }
}
/// `{"__repr__":` and `}` around the repr string of a value JSON cannot hold.
const REPR_WRAPPER_BYTES: usize = 13;
fn json_string_bytes(s: &str) -> usize {
    s.bytes()
        .map(|b| match b {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 2,
            0..=31 => 6,
            _ => 1,
        })
        .sum::<usize>()
        .saturating_add(2)
}
fn short(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
struct Limited {
    text: String,
    max: usize,
    truncated: bool,
}
impl Write for Limited {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let part = short(s, self.max.saturating_sub(self.text.len()));
        self.text.push_str(part);
        if part.len() != s.len() {
            self.truncated = true;
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}
pub fn bounded_repr(v: ObjectRef<'_>, max: usize) -> (String, bool) {
    fn render(v: ObjectRef<'_>, out: &mut Limited, depth: usize, nodes: &mut usize) -> fmt::Result {
        *nodes += 1;
        if depth > MAX_DEPTH || *nodes > MAX_NODES {
            out.truncated = true;
            return Err(fmt::Error);
        }
        match unstable::node(v) {
            MontyNode::List(ids) | MontyNode::Tuple(ids) => {
                out.write_str("[")?;
                for (i, id) in ids.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    render(unstable::child(v, *id), out, depth + 1, nodes)?;
                }
                out.write_str("]")
            }
            MontyNode::Set(ids) | MontyNode::FrozenSet(ids) => {
                let frozen = matches!(unstable::node(v), MontyNode::FrozenSet(_));
                match (frozen, ids.is_empty()) {
                    (false, true) => return out.write_str("set()"),
                    (true, true) => return out.write_str("frozenset()"),
                    (true, false) => out.write_str("frozenset({")?,
                    (false, false) => out.write_str("{")?,
                }
                for (i, id) in ids.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    render(unstable::child(v, *id), out, depth + 1, nodes)?;
                }
                out.write_str(if frozen { "})" } else { "}" })
            }
            MontyNode::Dict(pairs) => {
                out.write_str("{")?;
                for (i, (key, val)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    render(unstable::child(v, *key), out, depth + 1, nodes)?;
                    out.write_str(": ")?;
                    render(unstable::child(v, *val), out, depth + 1, nodes)?;
                }
                out.write_str("}")
            }
            MontyNode::String(s) => {
                out.write_str("'")?;
                for c in s.chars() {
                    for c in c.escape_debug() {
                        out.write_char(c)?;
                    }
                }
                out.write_str("'")
            }
            MontyNode::BigInt(i) if i.bits() > 8192 => {
                out.truncated = true;
                out.write_str("<large int>")
            }
            MontyNode::Repr(text) => out.write_str(text),
            _ => write!(out, "{v}"),
        }
    }
    let mut out = Limited {
        text: String::new(),
        max,
        truncated: false,
    };
    let _ = render(v, &mut out, 0, &mut 0);
    (out.text, out.truncated)
}
