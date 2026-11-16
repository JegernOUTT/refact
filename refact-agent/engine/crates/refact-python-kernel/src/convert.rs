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
pub struct Budget {
    nodes: usize,
    bytes: usize,
    max_nodes: usize,
    max_bytes: usize,
    pub truncated: bool,
}
impl Budget {
    pub fn new(max_nodes: u32, max_bytes: u64) -> Self {
        Self {
            nodes: 0,
            bytes: 0,
            max_nodes: max_nodes as usize,
            max_bytes: max_bytes.min(MAX_VALUE_BYTES) as usize,
            truncated: false,
        }
    }
    fn charge(&mut self, depth: usize, bytes: usize) -> bool {
        self.nodes += 1;
        self.bytes = self.bytes.saturating_add(bytes);
        let ok = depth <= MAX_DEPTH && self.nodes <= self.max_nodes && self.bytes <= self.max_bytes;
        self.truncated |= !ok;
        ok
    }
    pub fn convert(&mut self, v: ObjectRef<'_>) -> Value {
        if self.nodes >= self.max_nodes || self.bytes >= self.max_bytes {
            self.truncated = true;
            return json!({"__repr__":"<truncated>"});
        }
        match self.inner(v, 0) {
            Some(v) => v,
            None => {
                self.truncated = true;
                json!({"__repr__":bounded_repr(v, MAX_REPR_BYTES).0})
            }
        }
    }
    fn fallback(&mut self, v: ObjectRef<'_>) -> Value {
        let (repr, cut) = bounded_repr(
            v,
            MAX_REPR_BYTES.min(self.max_bytes.saturating_sub(self.bytes) / 6),
        );
        self.truncated |= cut;
        self.bytes = self.bytes.saturating_add(json_string_bytes(&repr));
        json!({"__repr__":repr})
    }
    fn inner(&mut self, v: ObjectRef<'_>, depth: usize) -> Option<Value> {
        if !self.charge(depth, 24) {
            return None;
        }
        Some(match v.type_name() {
            "NoneType" => Value::Null,
            "bool" => json!(v.as_bool().unwrap()),
            "int" => match v.as_int() {
                Some(i) => json!(i),
                None => self.fallback(v),
            },
            "float" => match v.as_float().filter(|f| f.is_finite()) {
                Some(f) => json!(f),
                None => self.fallback(v),
            },
            "str" => {
                let s = v.as_str().unwrap();
                if s.len() > self.max_bytes.saturating_sub(self.bytes)
                    || !self.charge(depth, json_string_bytes(s))
                {
                    return None;
                }
                json!(s)
            }
            "list" | "tuple" => {
                let (MontyNode::List(ids) | MontyNode::Tuple(ids)) = unstable::node(v) else {
                    unreachable!()
                };
                if ids.len() > self.max_nodes {
                    return None;
                }
                let mut out = Vec::new();
                for id in ids {
                    out.push(self.inner(unstable::child(v, *id), depth + 1)?);
                }
                Value::Array(out)
            }
            "dict" => {
                let MontyNode::Dict(pairs) = unstable::node(v) else {
                    unreachable!()
                };
                if pairs.len() > self.max_nodes {
                    return None;
                }
                let mut out = serde_json::Map::new();
                for (k, id) in pairs {
                    let k = unstable::child(v, *k);
                    let k = k.as_str()?;
                    if !self.charge(depth, json_string_bytes(k)) {
                        return None;
                    }
                    out.insert(k.into(), self.inner(unstable::child(v, *id), depth + 1)?);
                }
                Value::Object(out)
            }
            _ => self.fallback(v),
        })
    }
}
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
            MontyNode::List(ids)
            | MontyNode::Tuple(ids)
            | MontyNode::Set(ids)
            | MontyNode::FrozenSet(ids) => {
                out.write_str("[")?;
                for (i, id) in ids.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    render(unstable::child(v, *id), out, depth + 1, nodes)?;
                }
                out.write_str("]")
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
            MontyNode::None
            | MontyNode::Bool(_)
            | MontyNode::Int(_)
            | MontyNode::Float(_)
            | MontyNode::BigInt(_) => write!(out, "{v}"),
            _ => {
                out.truncated = true;
                write!(out, "<{}>", v.type_name())
            }
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
