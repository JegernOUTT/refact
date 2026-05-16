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
    fn test_yaml_str_present() {
        let v = make_map(&[("key", serde_yaml::Value::String("value".to_string()))]);
        assert_eq!(yaml_str(&v, "key"), "value");
    }

    #[test]
    fn test_yaml_str_missing() {
        let v = make_map(&[]);
        assert_eq!(yaml_str(&v, "key"), "");
    }

    #[test]
    fn test_yaml_str_list_present() {
        let seq = serde_yaml::Value::Sequence(vec![
            serde_yaml::Value::String("a".to_string()),
            serde_yaml::Value::String("b".to_string()),
        ]);
        let v = make_map(&[("list", seq)]);
        assert_eq!(yaml_str_list(&v, "list"), vec!["a", "b"]);
    }

    #[test]
    fn test_yaml_str_list_missing() {
        let v = make_map(&[]);
        assert!(yaml_str_list(&v, "list").is_empty());
    }

    #[test]
    fn test_yaml_bool_true() {
        let v = make_map(&[("flag", serde_yaml::Value::Bool(true))]);
        assert!(yaml_bool(&v, "flag", false));
    }

    #[test]
    fn test_yaml_bool_false() {
        let v = make_map(&[("flag", serde_yaml::Value::Bool(false))]);
        assert!(!yaml_bool(&v, "flag", true));
    }

    #[test]
    fn test_yaml_bool_missing_uses_default() {
        let v = make_map(&[]);
        assert!(yaml_bool(&v, "missing", true));
        assert!(!yaml_bool(&v, "missing", false));
    }
}
