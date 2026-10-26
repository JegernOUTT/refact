pub fn yaml_str(v: &serde_yaml::Value, key: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub fn yaml_str_list(v: &serde_yaml::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn yaml_bool(v: &serde_yaml::Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_map(pairs: &[(&str, serde_yaml::Value)]) -> serde_yaml::Value {
        let mut m = serde_yaml::Mapping::new();
        for (k, v) in pairs {
            m.insert(serde_yaml::Value::String(k.to_string()), v.clone());
        }
        serde_yaml::Value::Mapping(m)
    }

    #[test]
    fn test_yaml_bool_false() {
        let v = make_map(&[("flag", serde_yaml::Value::Bool(false))]);
        assert!(!yaml_bool(&v, "flag", true));
    }
}
