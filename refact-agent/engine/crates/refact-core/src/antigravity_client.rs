pub const ANTIGRAVITY_VERSION: &str = "1.1.18";
pub const ANTIGRAVITY_CLIENT_REVISION: &str = "968774718";

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

fn antigravity_user_agent_os(os: &str) -> &'static str {
    match os {
        "windows" => "windows",
        "macos" => "darwin",
        _ => "linux",
    }
}

fn antigravity_user_agent_arch(arch: &str) -> &'static str {
    match arch {
        "aarch64" => "arm64",
        _ => "amd64",
    }
}

pub fn antigravity_user_agent() -> String {
    antigravity_user_agent_for(std::env::consts::OS, std::env::consts::ARCH)
}

fn antigravity_user_agent_for(os: &str, arch: &str) -> String {
    format!(
        "antigravity/cli/{} (aidev_client; os_type={}; arch={}; cl={}; auth_method=consumer)",
        ANTIGRAVITY_VERSION,
        antigravity_user_agent_os(os),
        antigravity_user_agent_arch(arch),
        ANTIGRAVITY_CLIENT_REVISION,
    )
}

pub fn antigravity_headers() -> Vec<(String, String)> {
    vec![("User-Agent".to_string(), antigravity_user_agent())]
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
    fn headers_match_the_official_cli_fingerprint() {
        assert_eq!(
            antigravity_user_agent_for("linux", "x86_64"),
            "antigravity/cli/1.1.18 (aidev_client; os_type=linux; arch=amd64; cl=968774718; auth_method=consumer)"
        );
        assert_eq!(
            antigravity_user_agent_for("macos", "aarch64"),
            "antigravity/cli/1.1.18 (aidev_client; os_type=darwin; arch=arm64; cl=968774718; auth_method=consumer)"
        );
        let headers = antigravity_headers();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "User-Agent");
    }
}
