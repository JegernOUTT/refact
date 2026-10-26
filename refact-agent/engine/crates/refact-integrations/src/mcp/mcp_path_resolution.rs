use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ResolvedCommand {
    pub program: PathBuf,
    pub effective_path: String,
}

#[derive(Debug)]
pub struct CommandNotFoundError {
    pub command: String,
    pub full_command: String,
    pub effective_path: String,
    pub extra_dirs_checked: Vec<String>,
}

impl CommandNotFoundError {
    pub fn to_user_message(&self) -> String {
        format!(
            "Command '{}' not found.\nCommand: {}\nSearched PATH: {}\nExtra dirs checked: {}\nSuggestions:\n  \u{2022} Install the tool (e.g. `pip install uv` for uvx, `npm install -g` for npx tools)\n  \u{2022} Use an absolute path in the command field\n  \u{2022} Add the binary's directory to env.PATH in the integration config",
            self.command,
            self.full_command,
            self.effective_path,
            self.extra_dirs_checked.join(", "),
        )
    }
}

#[cfg(unix)]
const PATH_SEPARATOR: char = ':';
#[cfg(windows)]
const PATH_SEPARATOR: char = ';';

fn dedup_path_entries(entries: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .filter(|e| seen.insert(e.clone()))
        .collect()
}

/// Common directories where CLI tools are installed.
/// We intentionally keep this list short and universal.
/// Tool-specific version managers (nvm, volta, fnm, etc.) should be handled
/// by the user's shell profile or by setting env.PATH in the MCP YAML config.
fn extra_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(h) = home::home_dir() {
        dirs.push(h.join(".local/bin"));
        dirs.push(h.join(".cargo/bin"));
        dirs.push(h.join(".bun/bin"));
        dirs.push(h.join(".deno/bin"));
        dirs.push(h.join("go/bin"));
        dirs.push(h.join(".volta/bin"));
        dirs.push(h.join(".nvm/current/bin"));
        dirs.push(h.join(".local/share/fnm/aliases/default/bin"));

        #[cfg(windows)]
        {
            if let Ok(appdata) = std::env::var("APPDATA") {
                dirs.push(PathBuf::from(appdata).join("npm"));
            }
            if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
                dirs.push(PathBuf::from(localappdata).join("Programs").join("Python"));
            }
        }
    }

    #[cfg(unix)]
    {
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
    }

    dirs
}

pub fn augmented_path(base_path: Option<&str>) -> String {
    let base = base_path
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());

    let mut entries: Vec<String> = base
        .split(PATH_SEPARATOR)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    for dir in extra_dirs() {
        if dir.exists() {
            entries.push(dir.to_string_lossy().into_owned());
        }
    }

    dedup_path_entries(entries).join(&PATH_SEPARATOR.to_string())
}

pub fn resolve_command(
    argv0: &str,
    full_command: &str,
    config_env_path: Option<&str>,
) -> Result<ResolvedCommand, CommandNotFoundError> {
    let effective_path = augmented_path(config_env_path);

    let extra_dirs_display: Vec<String> = extra_dirs()
        .into_iter()
        .filter(|d| d.exists())
        .map(|d| {
            if let Some(ref home) = home::home_dir() {
                let rel = d
                    .strip_prefix(home)
                    .ok()
                    .map(|r| format!("~/{}", r.display()));
                rel.unwrap_or_else(|| d.display().to_string())
            } else {
                d.display().to_string()
            }
        })
        .collect();

    if Path::new(argv0).components().count() > 1 {
        let p = PathBuf::from(argv0);
        if p.exists() {
            return Ok(ResolvedCommand {
                program: p,
                effective_path,
            });
        }
        return Err(CommandNotFoundError {
            command: argv0.to_string(),
            full_command: full_command.to_string(),
            effective_path,
            extra_dirs_checked: extra_dirs_display,
        });
    }

    match which::which_in(argv0, Some(&effective_path), ".") {
        Ok(path) => Ok(ResolvedCommand {
            program: path,
            effective_path,
        }),
        Err(_) => Err(CommandNotFoundError {
            command: argv0.to_string(),
            full_command: full_command.to_string(),
            effective_path,
            extra_dirs_checked: extra_dirs_display,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn make_executable(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }

    #[cfg(windows)]
    fn make_executable(_path: &std::path::Path) {
        // On Windows, files don't need explicit execute permission
    }

    #[test]
    fn test_resolve_command_finds_binary_in_extra_dir() {
        let tmp = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let bin_name = "my_fake_tool_xyz.exe";
        #[cfg(not(windows))]
        let bin_name = "my_fake_tool_xyz";
        let bin_path = tmp.path().join(bin_name);
        std::fs::write(&bin_path, "#!/bin/sh\necho hi").unwrap();
        make_executable(&bin_path);

        let dir_str = tmp.path().to_str().unwrap();
        let result = resolve_command("my_fake_tool_xyz", "my_fake_tool_xyz --arg", Some(dir_str));
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved.program, bin_path);
    }
}
