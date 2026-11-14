use std::path::Path;

pub fn resolve(package: &str, override_version: Option<&str>) -> Result<String, String> {
    let version = override_version.unwrap_or(package);
    if !valid_semver(version) {
        return Err(format!(
            "invalid runtime version {version:?}: expected SemVer"
        ));
    }
    Ok(version.to_string())
}

fn valid_semver(version: &str) -> bool {
    let (version, build) = match version.split_once('+') {
        Some((v, b)) => (v, Some(b)),
        None => (version, None),
    };
    let (core, pre) = match version.split_once('-') {
        Some((v, p)) => (v, Some(p)),
        None => (version, None),
    };
    let numeric = |s: &str| {
        !s.is_empty()
            && s.bytes().all(|b| b.is_ascii_digit())
            && (s.len() == 1 || !s.starts_with('0'))
    };
    let parts: Vec<_> = core.split('.').collect();
    if parts.len() != 3 || !parts.iter().all(|p| numeric(p)) {
        return false;
    }
    for (suffix, prerelease) in [(pre, true), (build, false)] {
        if let Some(suffix) = suffix {
            if !suffix.split('.').all(|s| {
                !s.is_empty()
                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && (!prerelease || !s.bytes().all(|b| b.is_ascii_digit()) || numeric(s))
            }) {
                return false;
            }
        }
    }
    true
}

pub fn package_version(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "version" {
                    let value = value.trim();
                    let quote = value.chars().next()?;
                    if quote != '"' && quote != '\'' {
                        return None;
                    }
                    return value[1..].split_once(quote).map(|(v, _)| v.to_string());
                }
            }
        }
    }
    None
}

pub fn emit(engine_manifest: &Path) -> String {
    println!("cargo:rerun-if-env-changed=REFACT_BUILD_VERSION");
    println!("cargo:rerun-if-changed={}", engine_manifest.display());
    let text = std::fs::read_to_string(engine_manifest).expect("read engine Cargo.toml");
    let package = package_version(&text).expect("engine [package].version");
    let override_version = match std::env::var("REFACT_BUILD_VERSION") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => panic!("invalid REFACT_BUILD_VERSION: {error}"),
    };
    let version = resolve(&package, override_version.as_deref()).unwrap_or_else(|e| panic!("{e}"));
    println!("cargo:rustc-env=REFACT_ENGINE_VERSION={version}");
    version
}
