pub const MCP_TRANSPORT_PREFIXES: &[(&str, &str)] = &[
    ("stdio", "mcp_stdio_"),
    ("sse", "mcp_sse_"),
    ("http", "mcp_http_"),
];

pub fn config_prefix_for_transport(transport: &str) -> &'static str {
    match transport {
        "sse" => "mcp_sse_",
        "http" | "streamable-http" => "mcp_http_",
        _ => "mcp_stdio_",
    }
}

pub fn detect_transport(config_name: &str) -> String {
    for (transport, prefix) in MCP_TRANSPORT_PREFIXES {
        if config_name.starts_with(prefix) {
            return transport.to_string();
        }
    }
    "stdio".to_string()
}

pub fn shorten_config_name(yaml_stem: &str) -> String {
    for (_transport, prefix) in MCP_TRANSPORT_PREFIXES {
        if let Some(stripped) = yaml_stem.strip_prefix(prefix) {
            return format!("mcp_{}", stripped);
        }
    }
    yaml_stem.to_string()
}

pub fn validate_config_filename(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("config name must not be empty".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!(
            "config name '{}' contains invalid characters",
            name
        ));
    }
    if name.starts_with('/') || name.contains(':') {
        return Err(format!(
            "config name '{}' looks like an absolute path",
            name
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "config name '{}' contains unsafe characters (only a-z, A-Z, 0-9, _, - allowed)",
            name
        ));
    }
    if name.len() > 128 {
        return Err(format!("config name '{}' exceeds 128 characters", name));
    }
    Ok(())
}

pub fn validate_server_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("server id must not be empty".to_string());
    }
    if id.contains("..") || id.contains('\\') {
        return Err(format!("server id '{}' contains invalid characters", id));
    }
    if id.chars().any(|c| c.is_control()) {
        return Err(format!("server id '{}' contains control characters", id));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '/' || c == '.')
    {
        return Err(format!("server id '{}' contains unsafe characters", id));
    }
    if id.len() > 256 {
        return Err(format!("server id '{}' exceeds 256 characters", id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_config_filename_rejects_traversal() {
        assert!(validate_config_filename("../evil").is_err());
        assert!(validate_config_filename("foo/../../bar").is_err());
        assert!(validate_config_filename("").is_err());
        assert!(validate_config_filename("/etc/passwd").is_err());
        assert!(validate_config_filename("a\\b").is_err());
    }

    #[test]
    fn test_validate_config_filename_accepts_valid() {
        assert!(validate_config_filename("mcp_stdio_ok").is_ok());
        assert!(validate_config_filename("mcp_http_my-server").is_ok());
        assert!(validate_config_filename("my_server_123").is_ok());
        assert!(validate_config_filename("a-b-c").is_ok());
    }

    #[test]
    fn test_validate_server_id_allows_slash() {
        assert!(validate_server_id("owner/repo").is_ok());
        assert!(validate_server_id("github/github-mcp-server").is_ok());
        assert!(validate_server_id("namespace/name").is_ok());
    }

    #[test]
    fn test_validate_server_id_rejects_traversal() {
        assert!(validate_server_id("../evil").is_err());
        assert!(validate_server_id("a\\b").is_err());
        assert!(validate_server_id("").is_err());
    }

    #[test]
    fn test_config_prefix_roundtrip() {
        for (transport, prefix) in MCP_TRANSPORT_PREFIXES {
            assert_eq!(config_prefix_for_transport(transport), *prefix);
        }
        assert_eq!(config_prefix_for_transport("streamable-http"), "mcp_http_");
        assert_eq!(config_prefix_for_transport("unknown"), "mcp_stdio_");
    }

    #[test]
    fn test_shorten_config_name() {
        assert_eq!(shorten_config_name("mcp_stdio_github"), "mcp_github");
        assert_eq!(shorten_config_name("mcp_sse_myserver"), "mcp_myserver");
        assert_eq!(shorten_config_name("mcp_http_myserver"), "mcp_myserver");
        assert_eq!(
            shorten_config_name("other_integration"),
            "other_integration"
        );
    }

    #[test]
    fn test_detect_transport() {
        assert_eq!(detect_transport("mcp_stdio_github"), "stdio");
        assert_eq!(detect_transport("mcp_sse_myserver"), "sse");
        assert_eq!(detect_transport("mcp_http_myserver"), "http");
        assert_eq!(detect_transport("something_else"), "stdio");
    }
}
