use std::collections::HashSet;
use std::path::PathBuf;

use glob::Pattern;
use serde::{Deserialize, Serialize};

use crate::command_classify::{executable_basename, segment_command, CommandSegments};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiskEntry {
    pub id: String,
    pub exec: String,
    pub requires_flags: Vec<String>,
    pub requires_arg_globs: Vec<String>,
    pub level: RiskLevel,
    pub reason: String,
    pub escalate_outside_workspace: bool,
    pub requires_redirect: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiskFinding {
    pub entry_id: String,
    pub level: RiskLevel,
    pub reason: String,
    pub segment: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiskContext {
    pub workspace_roots: Vec<PathBuf>,
}

pub fn normalized_flags(argv: &[String]) -> HashSet<String> {
    let mut flags = HashSet::new();
    for arg in argv.iter().skip(1) {
        if arg == "--" {
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let name = long.split_once('=').map_or(long, |(name, _)| name);
            let normalized = match name {
                "recursive" => Some("-r"),
                "force" => Some("-f"),
                "all" => Some("-a"),
                "directory" => Some("-d"),
                "hard" => Some("--hard"),
                "remove" => Some("-r"),
                "delete" => Some("-d"),
                _ => None,
            };
            flags.insert(normalized.unwrap_or(arg.as_str()).to_string());
        } else if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 2 {
            for flag in arg[1..].chars() {
                flags.insert(format!("-{flag}"));
            }
        } else if arg.starts_with('-') {
            flags.insert(arg.clone());
        }
    }
    flags
}

pub fn is_outside_workspace(arg: &str, roots: &[PathBuf]) -> bool {
    if arg.starts_with('~') || arg == ".." || arg.starts_with("../") {
        return true;
    }
    let windows_absolute = arg.as_bytes().get(1) == Some(&b':')
        && arg.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        || arg.starts_with("\\\\");
    if !arg.starts_with('/') && !windows_absolute {
        return false;
    }
    let path = lexical_normalize(arg);
    !roots.iter().any(|root| {
        let root = lexical_normalize(&root.to_string_lossy());
        path == root
            || path
                .strip_prefix(&root)
                .is_some_and(|rest| root.ends_with('/') || rest.starts_with('/'))
    })
}

fn lexical_normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut prefix = String::new();
    let mut parts = Vec::new();
    let rest = if path.starts_with("//") {
        prefix.push_str("//");
        path.trim_start_matches('/')
    } else if path.as_bytes().get(1) == Some(&b':') {
        prefix.push_str(&path[..2].to_ascii_lowercase());
        path[2..].trim_start_matches('/')
    } else {
        prefix.push('/');
        path.trim_start_matches('/')
    };
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        prefix
    } else if prefix.ends_with('/') {
        format!("{prefix}{}", parts.join("/"))
    } else {
        format!("{prefix}/{}", parts.join("/"))
    }
}

pub fn classify_command(
    segments: &CommandSegments,
    catalogue: &[RiskEntry],
    ctx: &RiskContext,
) -> Option<RiskFinding> {
    let mut best: Option<RiskFinding> = None;
    for entry in catalogue {
        let exec_pattern = match Pattern::new(&entry.exec) {
            Ok(pattern) => pattern,
            Err(error) => {
                tracing::warn!("Invalid glob pattern '{}': {}", entry.exec, error);
                continue;
            }
        };
        let arg_patterns: Option<Vec<Pattern>> = entry
            .requires_arg_globs
            .iter()
            .map(|glob| match Pattern::new(glob) {
                Ok(pattern) => Some(pattern),
                Err(error) => {
                    tracing::warn!("Invalid glob pattern '{}': {}", glob, error);
                    None
                }
            })
            .collect();
        let Some(arg_patterns) = arg_patterns else {
            continue;
        };

        for segment in &segments.segments {
            if !executable_basename(segment).is_some_and(|exec| exec_pattern.matches(exec)) {
                continue;
            }
            let flags = normalized_flags(&segment.argv);
            if !entry.requires_flags.iter().all(|flag| flags.contains(flag)) {
                continue;
            }
            if entry.requires_redirect && !segment.argv.iter().any(|arg| arg == ">" || arg == ">>")
            {
                continue;
            }
            if !arg_patterns
                .iter()
                .all(|pattern| segment.argv.iter().skip(1).any(|arg| pattern.matches(arg)))
            {
                continue;
            }
            let mut level = entry.level;
            if entry.escalate_outside_workspace
                && segment
                    .argv
                    .iter()
                    .skip(1)
                    .any(|arg| is_outside_workspace(arg, &ctx.workspace_roots))
            {
                level = escalate(level);
            }
            if best.as_ref().map_or(true, |finding| level > finding.level) {
                best = Some(RiskFinding {
                    entry_id: entry.id.clone(),
                    level,
                    reason: entry.reason.clone(),
                    segment: segment_command(segment),
                });
            }
        }
    }
    best
}

fn escalate(level: RiskLevel) -> RiskLevel {
    match level {
        RiskLevel::Low => RiskLevel::Medium,
        RiskLevel::Medium => RiskLevel::High,
        RiskLevel::High | RiskLevel::Critical => RiskLevel::Critical,
    }
}

fn entry(
    id: &str,
    exec: &str,
    flags: &[&str],
    args: &[&str],
    level: RiskLevel,
    reason: &str,
    escalate: bool,
) -> RiskEntry {
    RiskEntry {
        id: id.to_string(),
        exec: exec.to_string(),
        requires_flags: flags.iter().map(|value| value.to_string()).collect(),
        requires_arg_globs: args.iter().map(|value| value.to_string()).collect(),
        level,
        reason: reason.to_string(),
        escalate_outside_workspace: escalate,
        requires_redirect: false,
    }
}

pub fn default_catalogue() -> Vec<RiskEntry> {
    use RiskLevel::{Critical, High, Low, Medium};
    vec![
        RiskEntry {
            id: "redirect.device".to_string(),
            exec: "*".to_string(),
            requires_flags: Vec::new(),
            requires_arg_globs: vec!["/dev/*".to_string()],
            level: Critical,
            reason: "Writes directly to a device node.".to_string(),
            escalate_outside_workspace: false,
            requires_redirect: true,
        },
        entry(
            "rm.root",
            "rm",
            &[],
            &["/"],
            Critical,
            "Removes the filesystem root.",
            true,
        ),
        entry(
            "rm.home",
            "rm",
            &["-r", "-f"],
            &["~*"],
            High,
            "Recursively removes files under a home directory.",
            true,
        ),
        entry(
            "rm.recursive_force",
            "rm",
            &["-r", "-f"],
            &[],
            Medium,
            "Recursively and forcibly removes files.",
            true,
        ),
        entry(
            "rm.recursive",
            "rm",
            &["-r"],
            &[],
            Medium,
            "Recursively removes files.",
            true,
        ),
        entry("rm.plain", "rm", &[], &[], Low, "Removes files.", true),
        entry(
            "shred.files",
            "shred",
            &[],
            &[],
            High,
            "Overwrites file contents irreversibly.",
            true,
        ),
        entry(
            "wipe.files",
            "wipe",
            &[],
            &[],
            High,
            "Erases files irreversibly.",
            true,
        ),
        entry(
            "srm.files",
            "srm",
            &[],
            &[],
            High,
            "Securely removes files.",
            true,
        ),
        entry(
            "dd.device_output",
            "dd",
            &[],
            &["of=/dev/*"],
            Critical,
            "Writes raw data directly to a device.",
            false,
        ),
        entry(
            "dd.output",
            "dd",
            &[],
            &["of=*"],
            High,
            "Writes raw data to an output target.",
            false,
        ),
        entry(
            "mkfs.device",
            "mkfs*",
            &[],
            &[],
            Critical,
            "Creates a filesystem and destroys existing data.",
            false,
        ),
        entry(
            "fdisk.device",
            "fdisk",
            &[],
            &[],
            Critical,
            "Changes a disk partition table.",
            false,
        ),
        entry(
            "parted.device",
            "parted",
            &[],
            &[],
            Critical,
            "Changes disk partitions.",
            false,
        ),
        entry(
            "mkswap.device",
            "mkswap",
            &[],
            &[],
            Critical,
            "Overwrites a device with swap metadata.",
            false,
        ),
        entry(
            "truncate.file",
            "truncate",
            &[],
            &[],
            High,
            "Changes file sizes and may destroy data.",
            true,
        ),
        entry(
            "git.push_force",
            "git",
            &["-f"],
            &["push"],
            High,
            "Force-pushes remote Git history.",
            false,
        ),
        entry(
            "git.push",
            "git",
            &[],
            &["push"],
            Medium,
            "Publishes commits to a remote repository.",
            false,
        ),
        entry(
            "git.reset_hard",
            "git",
            &["--hard"],
            &["reset"],
            High,
            "Discards working tree and index changes.",
            true,
        ),
        entry(
            "git.clean_force",
            "git",
            &["-f"],
            &["clean"],
            High,
            "Permanently removes untracked files.",
            true,
        ),
        entry(
            "git.clean_dirs",
            "git",
            &["-d", "-f"],
            &["clean"],
            High,
            "Permanently removes untracked directories.",
            true,
        ),
        entry(
            "git.clean_ignored",
            "git",
            &["-x", "-f"],
            &["clean"],
            High,
            "Removes ignored and untracked files.",
            true,
        ),
        entry(
            "git.rm",
            "git",
            &[],
            &["rm"],
            Medium,
            "Removes tracked files.",
            true,
        ),
        entry(
            "docker.rm",
            "docker",
            &[],
            &["rm"],
            Medium,
            "Removes containers.",
            false,
        ),
        entry(
            "docker.rmi",
            "docker",
            &[],
            &["rmi"],
            Medium,
            "Removes container images.",
            false,
        ),
        entry(
            "docker.system_prune",
            "docker",
            &[],
            &["system", "prune"],
            High,
            "Prunes unused container resources.",
            false,
        ),
        entry(
            "kubectl.delete",
            "kubectl",
            &[],
            &["delete"],
            High,
            "Deletes orchestration resources.",
            false,
        ),
        entry(
            "chmod.recursive",
            "chmod",
            &["-R"],
            &[],
            High,
            "Recursively changes file permissions.",
            true,
        ),
        entry(
            "chown.recursive",
            "chown",
            &["-R"],
            &[],
            High,
            "Recursively changes file ownership.",
            true,
        ),
        entry(
            "chmod.world_writable",
            "chmod",
            &[],
            &["777"],
            High,
            "Grants all permissions to every user.",
            false,
        ),
        entry(
            "chmod.symbolic_world",
            "chmod",
            &[],
            &["a+rwx"],
            High,
            "Grants all permissions to every user.",
            false,
        ),
        entry(
            "shutdown.system",
            "shutdown",
            &[],
            &[],
            Critical,
            "Shuts down the system.",
            false,
        ),
        entry(
            "reboot.system",
            "reboot",
            &[],
            &[],
            Critical,
            "Reboots the system.",
            false,
        ),
        entry(
            "halt.system",
            "halt",
            &[],
            &[],
            Critical,
            "Halts the system.",
            false,
        ),
        entry(
            "poweroff.system",
            "poweroff",
            &[],
            &[],
            Critical,
            "Powers off the system.",
            false,
        ),
        entry(
            "systemctl.stop",
            "systemctl",
            &[],
            &["stop"],
            High,
            "Stops a system service.",
            false,
        ),
        entry(
            "systemctl.disable",
            "systemctl",
            &[],
            &["disable"],
            High,
            "Disables a system service.",
            false,
        ),
        entry(
            "kill.sigkill",
            "kill",
            &[],
            &["-9"],
            High,
            "Forcibly terminates a process.",
            false,
        ),
        entry(
            "killall.processes",
            "killall",
            &[],
            &[],
            Medium,
            "Terminates processes by name.",
            false,
        ),
        entry(
            "pkill.processes",
            "pkill",
            &[],
            &[],
            Medium,
            "Terminates matching processes.",
            false,
        ),
        entry(
            "apt.remove",
            "apt*",
            &[],
            &["remove"],
            Medium,
            "Removes installed packages.",
            false,
        ),
        entry(
            "apt.purge",
            "apt*",
            &[],
            &["purge"],
            High,
            "Purges installed packages and configuration.",
            false,
        ),
        entry(
            "yum.remove",
            "yum",
            &[],
            &["remove"],
            Medium,
            "Removes installed packages.",
            false,
        ),
        entry(
            "yum.erase",
            "yum",
            &[],
            &["erase"],
            Medium,
            "Erases installed packages.",
            false,
        ),
        entry(
            "dnf.remove",
            "dnf",
            &[],
            &["remove"],
            Medium,
            "Removes installed packages.",
            false,
        ),
        entry(
            "dnf.erase",
            "dnf",
            &[],
            &["erase"],
            Medium,
            "Erases installed packages.",
            false,
        ),
        entry(
            "dnf.purge",
            "dnf",
            &[],
            &["purge"],
            High,
            "Purges installed packages.",
            false,
        ),
        entry(
            "yum.purge",
            "yum",
            &[],
            &["purge"],
            High,
            "Purges installed packages.",
            false,
        ),
        entry(
            "pacman.remove",
            "pacman",
            &["-R"],
            &[],
            Medium,
            "Removes installed packages.",
            false,
        ),
        entry(
            "brew.uninstall",
            "brew",
            &[],
            &["uninstall"],
            Medium,
            "Uninstalls packages.",
            false,
        ),
        entry(
            "mount.filesystem",
            "mount",
            &[],
            &[],
            Medium,
            "Mounts a filesystem.",
            false,
        ),
        entry(
            "umount.filesystem",
            "umount",
            &[],
            &[],
            High,
            "Unmounts a filesystem.",
            false,
        ),
        entry(
            "swapon.device",
            "swapon",
            &[],
            &[],
            High,
            "Enables a swap device.",
            false,
        ),
        entry(
            "swapoff.device",
            "swapoff",
            &[],
            &[],
            High,
            "Disables a swap device.",
            false,
        ),
        entry(
            "tee.device",
            "tee",
            &[],
            &["/dev/*"],
            Critical,
            "Writes directly to a device.",
            false,
        ),
        entry(
            "cp.device",
            "cp",
            &[],
            &["/dev/*"],
            Critical,
            "Copies data directly to a device.",
            false,
        ),
        entry(
            "mv.device",
            "mv",
            &[],
            &["/dev/*"],
            Critical,
            "Moves data directly to a device.",
            false,
        ),
        entry(
            "crontab.remove",
            "crontab",
            &["-r"],
            &[],
            High,
            "Removes the user's scheduled jobs.",
            false,
        ),
        entry(
            "history.clear",
            "history",
            &["-c"],
            &[],
            Medium,
            "Clears shell command history.",
            false,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_classify::extract_command_segments;

    fn classify(command: &str, roots: Vec<PathBuf>) -> Option<RiskFinding> {
        classify_command(
            &extract_command_segments(command),
            &default_catalogue(),
            &RiskContext {
                workspace_roots: roots,
            },
        )
    }

    #[test]
    fn recursive_rm_risk_is_path_aware() {
        assert_eq!(
            classify("rm -rf /", vec![]).unwrap().level,
            RiskLevel::Critical
        );
        let local = classify("rm -rf node_modules", vec![PathBuf::from("/workspace")]).unwrap();
        assert!(local.level <= RiskLevel::Medium);
    }

    #[test]
    fn rm_flag_spellings_are_equivalent() {
        let commands = ["rm -r -f x", "rm -rf x", "rm --recursive --force x"];
        let findings: Vec<_> = commands
            .iter()
            .map(|command| classify(command, vec![]).unwrap())
            .collect();
        assert!(findings.iter().all(|finding| {
            finding.entry_id == findings[0].entry_id && finding.level == findings[0].level
        }));
    }

    #[test]
    fn force_push_is_more_severe() {
        assert_eq!(
            classify("git push", vec![]).unwrap().level,
            RiskLevel::Medium
        );
        assert_eq!(
            classify("git push --force", vec![]).unwrap().level,
            RiskLevel::High
        );
    }

    #[test]
    fn common_commands_have_no_findings() {
        for command in [
            "git add .",
            "npm run format",
            "cargo test",
            "git status",
            "ls -la",
            "grep -E 'ChatForm|Dropzone'",
        ] {
            assert_eq!(classify(command, vec![]), None, "{command:?}");
        }
    }

    #[test]
    fn device_redirect_is_critical_but_device_read_is_not() {
        let finding = classify("echo x > /dev/sda", vec![]).unwrap();
        assert_eq!(finding.entry_id, "redirect.device");
        assert_eq!(finding.level, RiskLevel::Critical);
        assert_eq!(classify("cat /dev/null", vec![]), None);
    }

    #[test]
    fn workspace_paths_are_lexically_normalized() {
        let roots = vec![PathBuf::from("/workspace")];
        assert!(is_outside_workspace("/workspace/../etc/passwd", &roots));
        assert!(!is_outside_workspace("/workspace/sub/file", &roots));
    }
}
