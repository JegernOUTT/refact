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

/// Detects the MCP transport from raw user input: a URL means a remote
/// (streamable HTTP) server, anything else is treated as a stdio command.
/// This is the single source of truth shared by the marketplace auto-name
/// endpoint and the unified integration dispatch.
pub fn detect_transport_from_input(input: &str) -> &'static str {
    let trimmed = input.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        "http"
    } else {
        "stdio"
    }
}

/// Model-facing tool names must satisfy `^[a-zA-Z0-9_-]+$` and stay within
/// 64 bytes for OpenAI-style function calling.
pub const MAX_MODEL_TOOL_NAME_BYTES: usize = 64;

/// Stable FNV-1a 64-bit hash used for deterministic name disambiguation.
pub fn stable_name_hash(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Builds the model-facing name for an MCP tool: `{server}_{tool}` sanitized
/// to ASCII alphanumerics/underscores and capped at
/// [`MAX_MODEL_TOOL_NAME_BYTES`]. Over-long names keep a stable 8-hex hash of
/// the original pair so truncation cannot silently merge two tools.
pub fn model_tool_name(server_part: &str, tool_part: &str) -> String {
    let raw = format!("{}_{}", server_part, tool_part);
    let sanitized: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if sanitized.len() <= MAX_MODEL_TOOL_NAME_BYTES {
        return sanitized;
    }
    with_hash_suffix(&sanitized, &raw)
}

/// Appends a stable hash suffix (derived from `original`) to `candidate`,
/// keeping the result within [`MAX_MODEL_TOOL_NAME_BYTES`].
pub fn with_hash_suffix(candidate: &str, original: &str) -> String {
    let suffix = format!("_{:08x}", stable_name_hash(original) as u32);
    let keep = MAX_MODEL_TOOL_NAME_BYTES.saturating_sub(suffix.len());
    let mut trimmed = candidate.to_string();
    trimmed.truncate(keep);
    let trimmed = trimmed.trim_end_matches('_');
    format!("{}{}", trimmed, suffix)
}

/// Disambiguates a batch of candidate model tool names. Names that collide
/// after sanitization (e.g. `read.file` and `read_file`) receive a stable
/// hash suffix derived from their original raw tool name; unique names are
/// returned untouched so existing configurations keep working.
pub fn disambiguate_model_tool_names(candidates: Vec<(String, String)>) -> Vec<String> {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (candidate, _original) in &candidates {
        *counts.entry(candidate.as_str()).or_insert(0) += 1;
    }
    candidates
        .iter()
        .map(|(candidate, original)| {
            if counts[candidate.as_str()] > 1 {
                with_hash_suffix(candidate, original)
            } else {
                candidate.clone()
            }
        })
        .collect()
}

/// Extracts a snake_case server name suggestion from raw user input
/// (either a command line such as `npx -y @org/mcp-server` or a URL).
pub fn extract_name_from_input(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("input is empty".to_string());
    }

    let raw = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        extract_name_from_url(trimmed)
    } else {
        extract_name_from_command(trimmed)
    };

    let sanitized = sanitize_suggested_name(&raw);
    if sanitized.is_empty() {
        return Err("could not extract a valid name from input".to_string());
    }
    Ok(sanitized)
}

fn extract_name_from_url(url: &str) -> String {
    let without_scheme = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host_and_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host = if host_and_port.starts_with('[') {
        host_and_port
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or("mcp")
    } else {
        host_and_port.split(':').next().unwrap_or(host_and_port)
    };

    if host == "localhost" {
        return "localhost".to_string();
    }

    // IPv6 hosts arrive here with brackets already stripped (e.g. "::1").
    if host.contains(':') {
        return format!("ip_{}", host.replace(':', "_"));
    }
    let is_ipv4 = !host.is_empty()
        && host
            .split('.')
            .all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit()));
    if is_ipv4 {
        return host.replace('.', "_");
    }

    let parts: Vec<&str> = host.split('.').collect();
    // Country-code SLD pattern: e.g. example.co.uk, example.com.au
    // Only trigger when last segment is 2-char country code AND second-to-last is a short
    // known SLD (co, com, org, net, ac, gov, edu) — not for domains like mcp.myservice.io
    if parts.len() >= 3 {
        let last = parts[parts.len() - 1];
        let second_last = parts[parts.len() - 2];
        let is_country_code_sld = last.len() == 2
            && matches!(
                second_last,
                "co" | "com" | "org" | "net" | "ac" | "gov" | "edu" | "or" | "ne"
            );
        if is_country_code_sld {
            return parts[parts.len() - 3].to_string();
        }
    }
    if parts.len() >= 2 {
        parts[parts.len() - 2].to_string()
    } else {
        parts.first().copied().unwrap_or("mcp").to_string()
    }
}

fn extract_name_from_command(cmd: &str) -> String {
    let args: Vec<&str> = cmd.split_whitespace().collect();
    let mut candidate = "";
    for (i, arg) in args.iter().enumerate() {
        if *arg == "run" || *arg == "-y" || *arg == "-i" || *arg == "--rm" || *arg == "-it" {
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        if i > 0
            && (args[i - 1] == "-e"
                || args[i - 1] == "--env"
                || args[i - 1] == "-p"
                || args[i - 1] == "--port")
        {
            continue;
        }
        candidate = arg;
        if *arg != "npx"
            && *arg != "uvx"
            && *arg != "docker"
            && *arg != "node"
            && *arg != "python"
            && *arg != "python3"
        {
            break;
        }
    }
    let name = candidate.rsplit('/').next().unwrap_or(candidate);
    let name = name.trim_end_matches(".js");
    let name = name.trim_start_matches('@');
    let name = if let Some(slash_pos) = name.find('/') {
        &name[slash_pos + 1..]
    } else {
        name
    };
    strip_mcp_name_prefixes(name)
}

fn strip_mcp_name_prefixes(s: &str) -> String {
    let stripped = s
        .trim_start_matches("mcp-server-")
        .trim_start_matches("server-mcp-")
        .trim_start_matches("mcp-")
        .trim_start_matches("server-");
    stripped.to_string()
}

fn sanitize_suggested_name(s: &str) -> String {
    let snake: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let snake = snake.trim_matches('_').to_string();
    let snake: String = {
        let mut prev_underscore = false;
        snake
            .chars()
            .filter(|c| {
                if *c == '_' {
                    if prev_underscore {
                        return false;
                    }
                    prev_underscore = true;
                } else {
                    prev_underscore = false;
                }
                true
            })
            .collect()
    };
    if snake.len() > 40 {
        snake[..40].to_string()
    } else {
        snake
    }
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

    #[test]
    fn test_detect_transport_from_input() {
        assert_eq!(
            detect_transport_from_input("https://api.example.com/mcp"),
            "http"
        );
        assert_eq!(detect_transport_from_input("http://localhost:3231"), "http");
        assert_eq!(
            detect_transport_from_input("npx -y @modelcontextprotocol/server-github"),
            "stdio"
        );
        assert_eq!(
            detect_transport_from_input("  docker run -i mcp/fetch"),
            "stdio"
        );
    }

    #[test]
    fn test_extract_name_from_npx_command() {
        let name = extract_name_from_input("npx -y @notionhq/notion-mcp-server").unwrap();
        assert_eq!(name, "notion_mcp_server");
    }

    #[test]
    fn test_extract_name_from_uvx_command() {
        let name = extract_name_from_input("uvx mcp-server-fetch").unwrap();
        assert_eq!(name, "fetch");
    }

    #[test]
    fn test_extract_name_from_url_basic() {
        let name = extract_name_from_input("https://api.example.com/mcp").unwrap();
        assert_eq!(name, "example");
    }

    #[test]
    fn test_extract_name_from_url_country_code_tld() {
        assert_eq!(
            extract_name_from_input("https://mcp.example.co.uk/api").unwrap(),
            "example"
        );
        assert_eq!(
            extract_name_from_input("https://mcp.myservice.io/mcp").unwrap(),
            "myservice"
        );
    }

    #[test]
    fn test_extract_name_from_url_localhost_and_ips() {
        assert_eq!(
            extract_name_from_input("http://localhost:3231/sse").unwrap(),
            "localhost"
        );
        assert_eq!(
            extract_name_from_input("http://192.168.1.10:8080/mcp").unwrap(),
            "192_168_1_10"
        );
    }

    #[test]
    fn test_extract_name_from_url_ipv6() {
        assert_eq!(
            extract_name_from_input("http://[::1]:3000/mcp").unwrap(),
            "ip_1"
        );
        assert_eq!(
            extract_name_from_input("http://[2001:db8::5]/mcp").unwrap(),
            "ip_2001_db8_5"
        );
    }

    #[test]
    fn test_extract_name_from_docker_command() {
        let name = extract_name_from_input("docker run -i --rm mcp/server-github").unwrap();
        assert_eq!(name, "github");
    }

    #[test]
    fn test_extract_name_sanitization() {
        let name = extract_name_from_input("npx -y @my-org/my-cool-tool!").unwrap();
        assert_eq!(name, "my_cool_tool");
    }

    #[test]
    fn test_extract_name_empty_input() {
        assert!(extract_name_from_input("").is_err());
        assert!(extract_name_from_input("   ").is_err());
    }

    #[test]
    fn test_model_tool_name_plain() {
        assert_eq!(
            model_tool_name("mcp_github", "create-issue"),
            "mcp_github_create_issue"
        );
        assert_eq!(model_tool_name("mcp_fs", "read.file"), "mcp_fs_read_file");
    }

    #[test]
    fn test_model_tool_name_caps_long_names_with_stable_hash() {
        let long_tool = "a".repeat(100);
        let name = model_tool_name("mcp_server", &long_tool);
        assert!(
            name.len() <= MAX_MODEL_TOOL_NAME_BYTES,
            "got {}",
            name.len()
        );
        let again = model_tool_name("mcp_server", &long_tool);
        assert_eq!(name, again, "hash suffix must be deterministic");
        let other = model_tool_name("mcp_server", &format!("{}b", "a".repeat(99)));
        assert_ne!(name, other, "different tools must not merge after capping");
    }

    #[test]
    fn test_disambiguate_model_tool_names() {
        let candidates = vec![
            ("mcp_s_read_file".to_string(), "read.file".to_string()),
            ("mcp_s_read_file".to_string(), "read_file".to_string()),
            ("mcp_s_write_file".to_string(), "write_file".to_string()),
        ];
        let resolved = disambiguate_model_tool_names(candidates);
        assert_ne!(resolved[0], resolved[1], "colliding names must diverge");
        assert!(resolved[0].starts_with("mcp_s_read_file_"));
        assert!(resolved[1].starts_with("mcp_s_read_file_"));
        assert_eq!(resolved[2], "mcp_s_write_file", "unique names unchanged");
        for name in &resolved {
            assert!(name.len() <= MAX_MODEL_TOOL_NAME_BYTES);
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        }
    }
}
