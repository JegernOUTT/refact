pub const ANTIGRAVITY_VERSION: &str = "1.18.3";

pub fn antigravity_platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "WINDOWS",
        "macos" => "MACOS",
        _ => "LINUX",
    }
}

pub fn antigravity_user_agent_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => "windows/amd64",
        ("macos", "aarch64") => "darwin/arm64",
        ("macos", _) => "darwin/amd64",
        _ => "linux/amd64",
    }
}

pub fn antigravity_headers() -> Vec<(String, String)> {
    vec![
        (
            "User-Agent".to_string(),
            format!(
                "antigravity/{} {}",
                ANTIGRAVITY_VERSION,
                antigravity_user_agent_platform()
            ),
        ),
        (
            "X-Goog-Api-Client".to_string(),
            "google-cloud-sdk vscode_cloudshelleditor/0.1".to_string(),
        ),
        (
            "Client-Metadata".to_string(),
            format!(
                "{{\"ideType\":\"ANTIGRAVITY\",\"platform\":\"{}\",\"pluginType\":\"GEMINI\"}}",
                antigravity_platform()
            ),
        ),
    ]
}
