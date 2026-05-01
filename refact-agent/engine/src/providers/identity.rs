use crate::providers::registry::PROVIDER_NAMES;

pub const RESERVED_INSTANCE_IDS: &[&str] = &["defaults", "refact"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInstanceIdentity {
    pub instance_id: String,
    pub base_provider: String,
    pub display_name: Option<String>,
    pub wrap_instance: bool,
}

pub fn validate_provider_instance_id(instance_id: &str) -> Result<(), String> {
    if instance_id.is_empty() {
        return Err("provider instance id cannot be empty".to_string());
    }
    if instance_id.len() > 64 {
        return Err(format!(
            "provider instance id '{}' is too long (max 64 characters)",
            instance_id
        ));
    }
    if RESERVED_INSTANCE_IDS
        .iter()
        .any(|reserved| instance_id.eq_ignore_ascii_case(reserved))
    {
        return Err(format!(
            "provider instance id '{}' is reserved",
            instance_id
        ));
    }
    if instance_id.contains('/')
        || instance_id.contains('\\')
        || instance_id.contains('.')
        || instance_id.contains("..")
    {
        return Err(format!(
            "provider instance id '{}' contains invalid path characters",
            instance_id
        ));
    }

    let mut chars = instance_id.chars();
    let Some(first) = chars.next() else {
        return Err("provider instance id cannot be empty".to_string());
    };
    if !first.is_ascii_alphanumeric() {
        return Err(format!(
            "provider instance id '{}' must start with an ASCII letter or digit",
            instance_id
        ));
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
        return Err(format!(
            "provider instance id '{}' contains invalid characters",
            instance_id
        ));
    }

    Ok(())
}

pub fn provider_identity_from_yaml(
    instance_id: &str,
    yaml: &serde_yaml::Value,
) -> Result<ProviderInstanceIdentity, String> {
    validate_provider_instance_id(instance_id)?;

    let base_provider = match yaml.get("base_provider") {
        Some(value) => {
            let base_provider = value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    format!(
                        "provider instance '{}' has invalid base_provider",
                        instance_id
                    )
                })?;
            validate_base_provider(base_provider)?;
            base_provider.to_string()
        }
        None if is_known_provider_stem(instance_id) => instance_id.to_string(),
        None => {
            return Err(format!(
                "provider instance '{}' must set base_provider because its config stem is not a built-in provider name",
                instance_id
            ));
        }
    };

    let display_name = yaml
        .get("display_name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let wrap_instance = instance_id != base_provider || display_name.is_some();

    Ok(ProviderInstanceIdentity {
        instance_id: instance_id.to_string(),
        base_provider,
        display_name,
        wrap_instance,
    })
}

fn validate_base_provider(base_provider: &str) -> Result<(), String> {
    if !is_known_provider_stem(base_provider) {
        return Err(format!("unknown base provider '{}'", base_provider));
    }
    Ok(())
}

fn is_known_provider_stem(name: &str) -> bool {
    PROVIDER_NAMES.iter().any(|provider| *provider == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_instance_id_validation_accepts_expected_shape() {
        for id in ["openai", "openai_2", "openai-work", "A1_b-C"] {
            validate_provider_instance_id(id).unwrap();
        }
    }

    #[test]
    fn provider_instance_id_validation_rejects_invalid_values() {
        for id in [
            "",
            "_openai",
            "-openai",
            "defaults",
            "refact",
            "openai.2",
            "openai/2",
            "openai\\2",
            "openai 2",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert!(validate_provider_instance_id(id).is_err(), "{id}");
        }
    }

    #[test]
    fn legacy_builtin_stem_without_base_provider_resolves_to_self() {
        let yaml = serde_yaml::from_str("enabled: true").unwrap();
        let identity = provider_identity_from_yaml("openai", &yaml).unwrap();

        assert_eq!(identity.instance_id, "openai");
        assert_eq!(identity.base_provider, "openai");
        assert!(!identity.wrap_instance);
    }

    #[test]
    fn alias_stem_without_base_provider_is_rejected() {
        let yaml = serde_yaml::from_str("enabled: true").unwrap();

        assert!(provider_identity_from_yaml("openai_2", &yaml).is_err());
    }

    #[test]
    fn empty_base_provider_is_rejected() {
        let yaml = serde_yaml::from_str("base_provider: ''").unwrap();

        assert!(provider_identity_from_yaml("openai_2", &yaml).is_err());
    }
}
