pub const ANTIGRAVITY_VERSION: &str = "1.1.16";

pub fn antigravity_platform() -> &'static str {
    antigravity_platform_for(std::env::consts::OS, std::env::consts::ARCH)
}

fn antigravity_platform_for(os: &str, arch: &str) -> &'static str {
    match (os, arch) {
        ("windows", _) => "WINDOWS_AMD64",
        ("macos", "aarch64") => "DARWIN_ARM64",
        ("macos", _) => "DARWIN_AMD64",
        ("linux", "aarch64") => "LINUX_ARM64",
        ("linux", _) => "LINUX_AMD64",
        _ => "PLATFORM_UNSPECIFIED",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_metadata_uses_cloud_code_platform_enum_values() {
        assert_eq!(
            antigravity_platform_for("windows", "x86_64"),
            "WINDOWS_AMD64"
        );
        assert_eq!(antigravity_platform_for("macos", "x86_64"), "DARWIN_AMD64");
        assert_eq!(antigravity_platform_for("macos", "aarch64"), "DARWIN_ARM64");
        assert_eq!(antigravity_platform_for("linux", "x86_64"), "LINUX_AMD64");
        assert_eq!(antigravity_platform_for("linux", "aarch64"), "LINUX_ARM64");
        assert_eq!(
            antigravity_platform_for("freebsd", "x86_64"),
            "PLATFORM_UNSPECIFIED"
        );
    }

    #[test]
    fn client_metadata_uses_the_platform_enum() {
        let metadata = antigravity_headers()
            .into_iter()
            .find_map(|(name, value)| (name == "Client-Metadata").then_some(value))
            .unwrap();
        let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        let platform = metadata["platform"].as_str().unwrap();

        assert!(matches!(
            platform,
            "WINDOWS_AMD64"
                | "DARWIN_AMD64"
                | "DARWIN_ARM64"
                | "LINUX_AMD64"
                | "LINUX_ARM64"
                | "PLATFORM_UNSPECIFIED"
        ));
    }
}
