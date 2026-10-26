use std::collections::HashMap;

pub const MAX_TOOL_NAME_LEN: usize = 64;

fn is_provider_safe(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_TOOL_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && name
            .chars()
            .next()
            .map_or(false, |c| c.is_ascii_alphabetic())
}

pub fn generate_tool_alias(name: &str, max_len: usize) -> String {
    if is_provider_safe(name) && name.len() <= max_len {
        return name.to_string();
    }
    let hash = format!("{:x}", md5::compute(name.as_bytes()));
    let hash8 = &hash[..8];
    let prefix_len = max_len.saturating_sub(9);
    let prefix: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(prefix_len)
        .collect();
    let prefix = if prefix.is_empty()
        || !prefix
            .chars()
            .next()
            .map_or(false, |c| c.is_ascii_alphabetic())
    {
        format!("t_{}", &prefix)
    } else {
        prefix
    };
    format!("{}_{}", prefix, hash8)
}

#[derive(Clone)]
pub struct ToolAliasRegistry {
    name_to_alias: HashMap<String, String>,
    alias_to_name: HashMap<String, String>,
}

impl ToolAliasRegistry {
    pub fn new() -> Self {
        ToolAliasRegistry {
            name_to_alias: HashMap::new(),
            alias_to_name: HashMap::new(),
        }
    }

    pub fn register(&mut self, internal_name: &str) -> String {
        if let Some(alias) = self.name_to_alias.get(internal_name) {
            return alias.clone();
        }
        let mut candidate = generate_tool_alias(internal_name, MAX_TOOL_NAME_LEN);
        if self.alias_to_name.contains_key(&candidate)
            && self.alias_to_name[&candidate] != internal_name
        {
            let mut suffix = 1u32;
            loop {
                let suffixed = format!(
                    "{}_{}",
                    &candidate[..candidate.len().min(MAX_TOOL_NAME_LEN - 3)],
                    suffix
                );
                if !self.alias_to_name.contains_key(&suffixed) {
                    candidate = suffixed;
                    break;
                }
                suffix += 1;
            }
            tracing::warn!(
                "tool_name_alias: collision resolved: {} → {}",
                internal_name,
                candidate
            );
        }
        self.name_to_alias
            .insert(internal_name.to_string(), candidate.clone());
        self.alias_to_name
            .insert(candidate.clone(), internal_name.to_string());
        candidate
    }

    pub fn resolve_alias(&self, alias: &str) -> Option<&str> {
        self.alias_to_name.get(alias).map(|s| s.as_str())
    }

    pub fn get_alias(&self, internal_name: &str) -> Option<&str> {
        self.name_to_alias.get(internal_name).map(|s| s.as_str())
    }

    pub fn needs_aliasing(&self) -> bool {
        self.name_to_alias.iter().any(|(name, alias)| name != alias)
    }
}

pub fn build_registry_from_names(tool_names: &[String]) -> ToolAliasRegistry {
    let mut registry = ToolAliasRegistry::new();
    for name in tool_names {
        registry.register(name);
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_long_name_gets_truncated_with_hash() {
        let long_name =
            "some_extremely_long_tool_name_that_clearly_exceeds_the_sixty_four_character_limit";
        assert!(
            long_name.len() > 64,
            "test name should be longer than 64 chars"
        );
        let alias = generate_tool_alias(long_name, 64);
        assert!(alias.len() <= 64, "alias too long: {} chars", alias.len());
        assert!(
            alias.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "alias not provider-safe: {}",
            alias
        );
        assert_ne!(alias, long_name);
        assert!(alias
            .chars()
            .next()
            .map_or(false, |c| c.is_ascii_alphabetic()));
    }

    #[test]
    fn test_registry_roundtrip() {
        let mut registry = ToolAliasRegistry::new();
        let alias = registry.register("my_tool");
        assert_eq!(registry.resolve_alias(&alias), Some("my_tool"));
        assert_eq!(registry.get_alias("my_tool"), Some(alias.as_str()));
    }

    #[test]
    fn test_collision_resolution() {
        let mut registry = ToolAliasRegistry::new();
        let name1 = "server_a_do_something_very_special_and_unique_indeed_here";
        let name2 = "server_b_do_something_very_special_and_unique_indeed_here";
        let alias1 = registry.register(name1);
        let alias2 = registry.register(name2);
        assert_ne!(alias1, alias2, "Different tools must not share alias");
        assert_eq!(registry.resolve_alias(&alias1), Some(name1));
        assert_eq!(registry.resolve_alias(&alias2), Some(name2));
    }
}
