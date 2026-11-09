use serde::{Deserialize, Serialize};

/// Public reference metadata only. Never put credential values in a policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialBinding {
    pub reference: String,
    pub header: String,
    #[serde(default)]
    pub scheme: Option<String>,
    #[serde(default)]
    pub expected_identity_revision: Option<String>,
}

/// The host resolver must bind its private reference store to ALL these fields,
/// plus its own owner identity and scope version. References are not authority.
pub struct CredentialContext<'a> {
    pub app_id: &'a str,
    pub source_id: &'a str,
    pub config_digest: &'a str,
    pub origin: &'a str,
}

/// Injected by the trusted host only. Implementations must be memory-only here,
/// with no project/app-controlled environment, path, or secret-store lookup.
/// A revoked or unknown capability must fail, with a value-free error.
pub trait SourceCredentialResolver: Send + Sync {
    fn resolve(
        &self,
        context: &CredentialContext<'_>,
        binding: &CredentialBinding,
    ) -> Result<reqwest::header::HeaderValue, String>;
}

pub(crate) fn validate_binding(binding: &CredentialBinding) -> Result<(), String> {
    let name = reqwest::header::HeaderName::from_bytes(binding.header.as_bytes())
        .map_err(|_| "invalid credential header")?;
    // Do not allow credentials to alter routing, framing or stream semantics.
    if matches!(
        name.as_str(),
        "host"
            | "connection"
            | "content-length"
            | "transfer-encoding"
            | "upgrade"
            | "accept"
            | "last-event-id"
    ) || name.as_str().starts_with("sec-websocket-")
        || binding.reference.is_empty()
        || binding.reference.len() > 256
        || !binding
            .reference
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
        || binding.scheme.as_ref().is_some_and(|s| {
            s.is_empty()
                || s.len() > 32
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
    {
        return Err("invalid credential binding".into());
    }
    Ok(())
}

pub(crate) fn headers(
    resolver: Option<&dyn SourceCredentialResolver>,
    context: &CredentialContext<'_>,
    bindings: &[CredentialBinding],
) -> Result<reqwest::header::HeaderMap, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    for binding in bindings {
        validate_binding(binding)?;
        if binding
            .expected_identity_revision
            .as_ref()
            .is_none_or(|s| s.is_empty() || s.len() > 256)
        {
            return Err("credential identity revision required".into());
        }
        let raw = resolver
            .ok_or("credential resolver unavailable")?
            .resolve(context, binding)
            .map_err(|_| "credential capability denied")?;
        let mut value = if let Some(scheme) = &binding.scheme {
            reqwest::header::HeaderValue::from_bytes(
                &[scheme.as_bytes(), b" ", raw.as_bytes()].concat(),
            )
            .map_err(|_| "invalid resolved credential")?
        } else {
            raw
        };
        if value.as_bytes().len() > 8192 {
            return Err("credential size limit".into());
        }
        value.set_sensitive(true);
        let name = reqwest::header::HeaderName::from_bytes(binding.header.as_bytes())
            .map_err(|_| "invalid credential header")?;
        if headers.insert(name, value).is_some() {
            return Err("duplicate credential header".into());
        }
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Resolver;
    impl SourceCredentialResolver for Resolver {
        fn resolve(
            &self,
            c: &CredentialContext<'_>,
            b: &CredentialBinding,
        ) -> Result<reqwest::header::HeaderValue, String> {
            if c.app_id != "owner" || c.config_digest != "v1" || b.reference != "private-ref" {
                return Err("denied".into());
            }
            Ok(reqwest::header::HeaderValue::from_static("private-secret"))
        }
    }
    #[test]
    fn capabilities_fail_closed_and_debug_is_redacted() {
        let binding = CredentialBinding {
            reference: "private-ref".into(),
            header: "authorization".into(),
            scheme: Some("Bearer".into()),
            expected_identity_revision: Some("1".into()),
        };
        let mut context = CredentialContext {
            app_id: "owner",
            source_id: "source",
            config_digest: "v1",
            origin: "https://example.com",
        };
        assert!(headers(None, &context, &[binding.clone()]).is_err());
        let h = headers(Some(&Resolver), &context, &[binding.clone()]).unwrap();
        assert_eq!(h["authorization"], "Bearer private-secret");
        assert!(!format!("{h:?}").contains("private-secret"));
        let mut missing = binding.clone();
        missing.expected_identity_revision = None;
        assert!(headers(Some(&Resolver), &context, &[missing]).is_err());
        let mut routing = binding.clone();
        routing.header = "host".into();
        assert!(headers(Some(&Resolver), &context, &[routing]).is_err());
        context.app_id = "other";
        assert!(headers(Some(&Resolver), &context, &[binding]).is_err());
    }
}
