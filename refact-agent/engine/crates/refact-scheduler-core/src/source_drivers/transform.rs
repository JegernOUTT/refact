use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Immutable, digest-covered declarative transform. No expressions, code, I/O,
/// joins or mutable windows. Provenance belongs to the enclosing event and must
/// never be replaced with this projected payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformSpec {
    pub name: String,
    pub version: u32,
    #[serde(default)]
    pub select_fields: Vec<String>,
    #[serde(default)]
    pub rename: BTreeMap<String, String>,
    #[serde(default)]
    pub filter_equals: BTreeMap<String, Value>,
}
impl TransformSpec {
    pub fn validate(&self) -> Result<(), String> {
        refact_apps_core::validate_identifier(&self.name)?;
        if self.version != 1
            || self.select_fields.len() > 128
            || self.rename.len() > 128
            || self.filter_equals.len() > 32
        {
            return Err("transform definition limit or unsupported version".into());
        }
        for field in self
            .select_fields
            .iter()
            .chain(self.rename.keys())
            .chain(self.rename.values())
            .chain(self.filter_equals.keys())
        {
            if field.is_empty() || field.len() > 256 {
                return Err("transform field limit".into());
            }
        }
        bounded_definition(&serde_json::to_value(self).map_err(|_| "transform serialization")?)
    }
}
fn bounded_definition(value: &Value) -> Result<(), String> {
    let mut stack = vec![(value, 0usize)];
    let mut nodes = 0;
    while let Some((v, depth)) = stack.pop() {
        nodes += 1;
        if nodes > 8192 || depth > 64 {
            return Err("transform node/depth limit".into());
        }
        match v {
            Value::Array(a) => {
                if a.len() + stack.len() + nodes > 8192 {
                    return Err("transform node limit".into());
                }
                stack.extend(a.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(o) => {
                if o.len() + stack.len() + nodes > 8192 {
                    return Err("transform node limit".into());
                }
                stack.extend(o.values().map(|v| (v, depth + 1)));
            }
            _ => {}
        }
    }
    if serde_json::to_vec(value)
        .map_err(|_| "transform serialization")?
        .len()
        > 262_144
    {
        return Err("transform byte limit".into());
    }
    Ok(())
}
fn bounded(value: &Value) -> Result<(), String> {
    enum Children<'a> {
        Array(std::slice::Iter<'a, Value>),
        Object(serde_json::map::Values<'a>),
    }
    impl<'a> Iterator for Children<'a> {
        type Item = &'a Value;
        fn next(&mut self) -> Option<Self::Item> {
            match self {
                Self::Array(values) => values.next(),
                Self::Object(values) => values.next(),
            }
        }
    }
    let mut stack = Vec::new();
    let mut current = Some(value);
    loop {
        if let Some(value) = current.take() {
            if stack.len() > 64 {
                return Err("transform depth limit".into());
            }
            match value {
                Value::Array(values) => stack.push(Children::Array(values.iter())),
                Value::Object(values) => stack.push(Children::Object(values.values())),
                _ => {}
            }
        }
        match stack.last_mut() {
            Some(children) => match children.next() {
                Some(value) => current = Some(value),
                None => {
                    stack.pop();
                }
            },
            None => return Ok(()),
        }
    }
}
/// A failed predicate returns Null (a documented filtered-out sentinel); callers
/// that need to suppress delivery must test it explicitly. No stateful dedup.
pub fn apply_transform(spec: &TransformSpec, input: &Value) -> Result<Value, String> {
    spec.validate()?;
    bounded(input)?;
    let object = input.as_object().ok_or("transform requires object")?;
    if spec
        .filter_equals
        .iter()
        .any(|(k, v)| object.get(k) != Some(v))
    {
        return Ok(Value::Null);
    }
    let mut result = serde_json::Map::new();
    for (key, value) in object {
        if !spec.select_fields.is_empty() && !spec.select_fields.contains(key) {
            continue;
        }
        let target = spec.rename.get(key).unwrap_or(key);
        if result.insert(target.clone(), value.clone()).is_some() {
            return Err("transform rename collision".into());
        }
    }
    let output = Value::Object(result);
    bounded(&output)?;
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn declarative_projection_filter_and_limits() {
        let spec: TransformSpec = serde_json::from_value(json!({"name":"project","version":1,"select_fields":["x"],"rename":{"x":"value"},"filter_equals":{"kind":"metric"}})).unwrap();
        assert_eq!(
            apply_transform(&spec, &json!({"x":2,"kind":"metric","secret":"omitted"})).unwrap(),
            json!({"value":2})
        );
        assert_eq!(
            apply_transform(&spec, &json!({"kind":"other"})).unwrap(),
            Value::Null
        );
        let history = json!({"x": (0..10000).map(|i| json!({"at":i,"value":"a".repeat(40)})).collect::<Vec<_>>(), "kind":"metric"});
        assert_eq!(
            apply_transform(&spec, &history).unwrap(),
            json!({"value":history["x"]})
        );
        let mut deep = Value::Null;
        for _ in 0..65 {
            deep = json!([deep]);
        }
        assert!(apply_transform(&spec, &json!({"x":deep})).is_err());
        let mut collision = spec.clone();
        collision.select_fields.clear();
        collision.filter_equals.clear();
        collision.rename.insert("kind".into(), "value".into());
        assert!(apply_transform(&collision, &json!({"x":1,"kind":2})).is_err());
    }
}
