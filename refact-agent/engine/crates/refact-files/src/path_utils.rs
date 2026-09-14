use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use serde::Deserialize;
use tokio::process::Command;

use refact_core::custom_error::MapErrToString;

use crate::correction_cache::CacheCorrection;

#[cfg(windows)]
pub fn preprocess_path_for_normalization(p: String) -> String {
    use itertools::Itertools;

    let p = p.replace(r"/", r"\");
    let starting_slashes = p.chars().take_while(|c| *c == '\\').count();

    let mut parts_iter = p.split(r"\").filter(|part| !part.is_empty()).peekable();

    match parts_iter.peek() {
        Some(&"?") => {
            parts_iter.next();
            match parts_iter.peek() {
                Some(pref) if pref.contains(":") => parts_iter.join(r"\"),
                Some(pref) if pref.to_lowercase() == "unc" => {
                    parts_iter.next();
                    format!(r"\\{}", parts_iter.join(r"\"))
                }
                Some(_) => {
                    tracing::warn!(
                        "Found a verbatim path that is not UNC nor Disk path: {}, leaving it as-is",
                        p
                    );
                    p
                }
                None => p,
            }
        }
        Some(&".") if starting_slashes > 0 => {
            parts_iter.next();
            format!(r"\\.\{}", parts_iter.join(r"\"))
        }
        Some(pref) if pref.contains(":") => parts_iter.join(r"\"),
        Some(_) => match starting_slashes {
            0 => parts_iter.join(r"\"),
            1 => format!(r"\{}", parts_iter.join(r"\")),
            _ => format!(r"\\{}", parts_iter.join(r"\")),
        },
        None => p,
    }
}

#[cfg(not(windows))]
pub fn preprocess_path_for_normalization(p: String) -> String {
    p
}

#[cfg(windows)]
fn absolute(path: &Path) -> Result<PathBuf, String> {
    use std::path::Prefix;
    use std::ffi::OsString;

    let path = std::path::absolute(path).map_err_to_string()?;

    if let Some(Component::Prefix(pref)) = path.components().next() {
        match pref.kind() {
            Prefix::Disk(_) => {
                let mut path_os_str = OsString::from(r"\\?\");
                path_os_str.push(path.as_os_str());
                Ok(PathBuf::from(path_os_str))
            }
            Prefix::UNC(_, _) => {
                let mut path_os_str = OsString::from(r"\\?\UNC\");
                path_os_str.push(path.strip_prefix(r"\\").unwrap_or(&path).as_os_str());
                Ok(PathBuf::from(path_os_str))
            }
            _ => Ok(path.to_path_buf()),
        }
    } else {
        Ok(path.to_path_buf())
    }
}

#[cfg(not(windows))]
fn absolute(path: &Path) -> Result<PathBuf, String> {
    let mut components = path.components();
    let path_os = path.as_os_str().as_encoded_bytes();

    let mut normalized = if path.is_absolute() {
        if path_os.starts_with(b"//") && !path_os.starts_with(b"///") {
            components.next();
            PathBuf::from("//")
        } else {
            PathBuf::from("/")
        }
    } else {
        std::env::current_dir().map_err_to_string()?
    };
    for component in components {
        match component {
            Component::Normal(c) => {
                normalized.push(c);
            }
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => (),
            Component::RootDir => (),
            Component::Prefix(_) => return Err("Prefix should not occur in Unix".to_string()),
        }
    }

    if path_os.ends_with(b"/") {
        normalized.push("");
    }

    Ok(normalized)
}

pub fn canonical_path<T: Into<String>>(p: T) -> PathBuf {
    let p: String = p.into();
    let path = PathBuf::from(preprocess_path_for_normalization(p));
    canonicalize_normalized_path(path)
}

pub fn canonicalize_normalized_path(p: PathBuf) -> PathBuf {
    p.canonicalize()
        .map(|path| dunce::simplified(&path).to_path_buf())
        .unwrap_or_else(|_| {
            absolute(&p)
                .map(|path| dunce::simplified(&path).to_path_buf())
                .unwrap_or(p)
        })
}

const GLOB_PATTERN_CACHE_CAPACITY: usize = 1024;

static GLOB_PATTERN_CACHE: OnceLock<RwLock<HashMap<String, Option<Arc<glob::Pattern>>>>> =
    OnceLock::new();
static GLOB_PATTERN_COMPILATIONS: AtomicU64 = AtomicU64::new(0);

pub fn glob_pattern_compile_count() -> u64 {
    GLOB_PATTERN_COMPILATIONS.load(Ordering::Relaxed)
}

fn compiled_glob_pattern(glob: &str) -> Option<Arc<glob::Pattern>> {
    let cache = GLOB_PATTERN_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(cached) = cache
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(glob)
    {
        return cached.clone();
    }
    let compiled = match glob::Pattern::new(glob) {
        Ok(pattern) => Some(Arc::new(pattern)),
        Err(error) => {
            tracing::warn!("Invalid glob pattern '{}': {}", glob, error);
            None
        }
    };
    GLOB_PATTERN_COMPILATIONS.fetch_add(1, Ordering::Relaxed);
    let mut cache = cache
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() >= GLOB_PATTERN_CACHE_CAPACITY {
        cache.clear();
    }
    cache.insert(glob.to_string(), compiled.clone());
    compiled
}

pub fn any_glob_matches_path(globs: &[String], path: &Path) -> bool {
    globs.iter().any(|glob| {
        let Some(pattern) = compiled_glob_pattern(glob) else {
            return false;
        };
        pattern.matches_path(path) || path.to_str().is_some_and(|s: &str| s.ends_with(glob))
    })
}

pub fn serialize_path<S: serde::Serializer>(
    path: &PathBuf,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

pub fn deserialize_path<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<PathBuf, D::Error> {
    Ok(PathBuf::from(String::deserialize(deserializer)?))
}

pub trait CommandSimplifiedDirExt {
    fn current_dir_simplified<P: AsRef<Path>>(&mut self, dir: P) -> &mut Self;
}

impl CommandSimplifiedDirExt for Command {
    fn current_dir_simplified<P: AsRef<Path>>(&mut self, dir: P) -> &mut Self {
        self.current_dir(dunce::simplified(dir.as_ref()))
    }
}

pub fn shortify_paths_from_indexed(
    cache_correction: &CacheCorrection,
    paths: &[String],
) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            if let Some(shortened) = cache_correction.filenames.short_path(&PathBuf::from(path)) {
                return shortened.to_string_lossy().to_string();
            }
            if let Some(shortened) = cache_correction
                .directories
                .short_path(&PathBuf::from(path))
            {
                return shortened.to_string_lossy().to_string();
            }
            path.clone()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::correction_cache::CacheCorrection;

    #[test]
    fn test_any_glob_matches_path_rejects_malformed_glob() {
        assert!(!any_glob_matches_path(&["[".to_string()], Path::new("/a")));
    }

    #[test]
    fn target_sibling_build_dirs_need_the_hyphen_glob() {
        let target = Path::new("__root__/refact-agent/engine/target/__probe__");
        let target_e2e = Path::new("__root__/refact-agent/engine/target-e2e/__probe__");
        let target_e2e_debug = Path::new("__root__/refact-agent/engine/target-e2e/debug/__probe__");
        let targeting = Path::new("__root__/refact-agent/engine/targeting/__probe__");
        let before_fix = vec!["*/target/*".to_string()];
        let after_fix = vec!["*/target/*".to_string(), "*/target-*/*".to_string()];

        assert!(any_glob_matches_path(&before_fix, target));
        assert!(
            !any_glob_matches_path(&before_fix, target_e2e),
            "this is the gap the extra glob closes"
        );

        assert!(any_glob_matches_path(&after_fix, target));
        assert!(any_glob_matches_path(&after_fix, target_e2e));
        assert!(any_glob_matches_path(&after_fix, target_e2e_debug));
        assert!(
            !any_glob_matches_path(&after_fix, targeting),
            "a source dir whose name merely starts with target must stay indexed"
        );
    }

    #[test]
    fn glob_patterns_compiled_once_per_generation() {
        let path = Path::new("/repo/refact-agent/engine/target-e2e/debug/app");
        let globs = vec![
            "[".to_string(),
            "*/target/*".to_string(),
            "*/target-*/*".to_string(),
        ];

        assert!(any_glob_matches_path(&globs, path));
        let compilations_after_warmup = glob_pattern_compile_count();

        for _ in 0..256 {
            assert!(any_glob_matches_path(&globs, path));
        }

        assert_eq!(glob_pattern_compile_count(), compilations_after_warmup);
    }

    #[test]
    fn test_shortify_paths_from_indexed() {
        let workspace_folders = vec![
            PathBuf::from("home").join("user").join("repo1"),
            PathBuf::from("home")
                .join("user")
                .join("repo1")
                .join("nested")
                .join("repo2"),
            PathBuf::from("home").join("user").join("repo3"),
        ];

        let indexed_paths = vec![
            PathBuf::from("home")
                .join("user")
                .join("repo1")
                .join("dir")
                .join("file.ext"),
            PathBuf::from("home")
                .join("user")
                .join("repo1")
                .join("nested")
                .join("repo2")
                .join("dir")
                .join("file.ext"),
            PathBuf::from("home")
                .join("user")
                .join("repo3")
                .join("dir")
                .join("file.ext"),
            PathBuf::from("home")
                .join("user")
                .join("repo1")
                .join("this_file.ext"),
            PathBuf::from("home")
                .join("user")
                .join("repo1")
                .join(".hidden")
                .join("custom_dir")
                .join("file.ext"),
            PathBuf::from("home")
                .join("user")
                .join("repo3")
                .join("dir2")
                .join("another_file.ext"),
        ];

        let paths = vec![
            PathBuf::from("home")
                .join("user")
                .join("repo1")
                .join("dir")
                .join("file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("home")
                .join("user")
                .join("repo1")
                .join("nested")
                .join("repo2")
                .join("dir")
                .join("file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("home")
                .join("user")
                .join("repo3")
                .join("dir")
                .join("file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("home")
                .join("user")
                .join("repo3")
                .join("dir2")
                .join("another_file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("home")
                .join("user")
                .join("repo4")
                .join(".hidden")
                .join("custom_dir")
                .join("file.ext")
                .to_string_lossy()
                .to_string(),
        ];

        let cache_correction = CacheCorrection::build(&indexed_paths, &workspace_folders);
        let mut result = shortify_paths_from_indexed(&cache_correction, &paths);

        let mut expected_result = vec![
            PathBuf::from("repo1")
                .join("dir")
                .join("file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("nested")
                .join("repo2")
                .join("dir")
                .join("file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("repo3")
                .join("dir")
                .join("file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("dir2")
                .join("another_file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("home")
                .join("user")
                .join("repo4")
                .join(".hidden")
                .join("custom_dir")
                .join("file.ext")
                .to_string_lossy()
                .to_string(),
        ];

        result.sort();
        expected_result.sort();

        assert_eq!(
            result, expected_result,
            "The result should contain the expected paths, instead it found"
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_preprocess_windows_path_for_normalization() {
        let test_cases = [
            (
                r"\\\\\\\\?\\\\C:\\\\Windows\\\\System32",
                r"C:\Windows\System32",
            ),
            (
                r"\?\C:\Model generates this kind of paths",
                r"C:\Model generates this kind of paths",
            ),
            (r"/?/C:/other\\horr.ible/path", r"C:\other\horr.ible\path"),
            (r"C:\\folder/..\\\\file", r"C:\folder\..\file"),
            (
                r"/D:\\Users/John Doe\\\\.\myfolder/file.ext",
                r"D:\Users\John Doe\.\myfolder\file.ext",
            ),
            (
                r"\\?\UNC\server\share/folder//file.ext",
                r"\\server\share\folder\file.ext",
            ),
            (
                r"\\?\unc\server\share/folder//file.ext",
                r"\\server\share\folder\file.ext",
            ),
            (
                r"/?/unc/server/share/folder//file.ext",
                r"\\server\share\folder\file.ext",
            ),
            (
                r"\\server\share/folder//file.ext",
                r"\\server\share\folder\file.ext",
            ),
            (
                r"////server//share//folder//file.ext",
                r"\\server\share\folder\file.ext",
            ),
            (
                r"//wsl$/Ubuntu/home/yourusername/projects",
                r"\\wsl$\Ubuntu\home\yourusername\projects",
            ),
            (r"////./pipe/docker_engine", r"\\.\pipe\docker_engine"),
            (r"\\.\pipe\docker_engine", r"\\.\pipe\docker_engine"),
            (r"//./pipe/docker_engine", r"\\.\pipe\docker_engine"),
            (r"\Windows\System32", r"\Windows\System32"),
            (
                r"/Program Files/Common Files",
                r"\Program Files\Common Files",
            ),
            (r"\Users\Public\Downloads", r"\Users\Public\Downloads"),
            (r"\temp/path", r"\temp\path"),
            (r"folder/file.txt", r"folder\file.txt"),
            (r"./current/./folder", r".\current\.\folder"),
            (r"project/../src/main.rs", r"project\..\src\main.rs"),
            (r"documents\\photos", r"documents\photos"),
            (
                r"some folder/with spaces/file",
                r"some folder\with spaces\file",
            ),
            (r"bin/../lib/./include", r"bin\..\lib\.\include"),
        ];

        for (input, expected) in test_cases {
            let result = preprocess_path_for_normalization(input.to_string());
            assert_eq!(
                result,
                expected.to_string(),
                "The result for {} should be {}, got {}",
                input,
                expected,
                result
            );
        }
    }

    #[cfg(windows)]
    #[ignore]
    #[test]
    fn test_canonical_path_windows() {
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_dir_path = temp_dir.path();
        let temp_dir_path_str = temp_dir_path.to_str().unwrap();

        let long_str = String::from_utf8(vec![b'a'; 600].iter().map(|b| *b).collect()).unwrap();
        let long_dir_path = PathBuf::from(format!("\\\\?\\{temp_dir_path_str}\\{long_str}"));

        let create_dir_cmd = format!(
            "powershell.exe -Command \"New-Item -Path '{}' -ItemType Directory -Force\"",
            long_dir_path.to_string_lossy().replace("'", "''")
        );
        let create_file_cmd = format!(
            "powershell.exe -Command \"New-Item -Path '{}' -ItemType File -Force\"",
            long_dir_path
                .join("file.txt")
                .to_string_lossy()
                .replace("'", "''")
        );
        std::process::Command::new("cmd")
            .args(["/C", &create_dir_cmd])
            .output()
            .expect("Failed to create directory");
        std::process::Command::new("cmd")
            .args(["/C", &create_file_cmd])
            .output()
            .expect("Failed to create file");

        let long_dir_path_str = format!("{temp_dir_path_str}\\{long_str}\\..\\{long_str}");
        let long_dir_file_str =
            format!("{temp_dir_path_str}\\{long_str}\\..\\{long_str}\\.\\..\\{long_str}\\file.txt");

        let test_cases = vec![
            (
                r"C:\\Windows\\System32\\..\\..\\Temp\\conn",
                PathBuf::from(r"\\?\C:\Temp\conn"),
            ),
            (r"D:/../..\NUL", PathBuf::from(r"\\.\NUL")),
            (
                r"d:\\A\\B\\C\\D\\..\\..\\..\\..\\E\\F\\G\\..\\..\\H",
                PathBuf::from(r"\\?\D:\E\H"),
            ),
            (r"c:\\../Windows", PathBuf::from(r"\\?\C:\Windows")),
            (r"d:\\..\\..\\..\\..\\..", PathBuf::from(r"\\?\D:\")),
            (
                r"\\\\?\\C:\Very\Long\Path\With\Lots\Of\Subdirectories\..\..\..\LongFile",
                PathBuf::from(r"\\?\C:\Very\Long\Path\With\LongFile"),
            ),
            (
                r"//?/d:/Trailing/Dot./.",
                PathBuf::from(r"\\?\d:\Trailing\Dot"),
            ),
            (
                r"\?\c:\Trailing\Space\\  ",
                PathBuf::from(r"\\?\c:\Trailing\Space\"),
            ),
            (r"\?/C:/$MFT", PathBuf::from(r"\\?\C:\$MFT")),
            (r"\\.\COM1", PathBuf::from(r"\\.\COM1")),
            (
                r"\.\PIPE\SomePipeName",
                PathBuf::from(r"\\.\PIPE\SomePipeName"),
            ),
            (
                r"/?/UNC//./PIPE/AnotherPipe",
                PathBuf::from(r"\\.\PIPE\AnotherPipe"),
            ),
            (
                r"\\?\Volume{12345678-1234-1234-1234-1234567890AB}\Path\To\Some\File",
                PathBuf::from(
                    r"\\?\Volume{12345678-1234-1234-1234-1234567890AB}\Path\To\Some\File",
                ),
            ),
            (
                r"\\?\UNC\localhost\C$/Windows/System32\..\System32",
                PathBuf::from(r"\\?\UNC\localhost\C$\Windows\System32"),
            ),
            (
                &long_dir_path_str,
                PathBuf::from(format!("\\\\?\\{temp_dir_path_str}\\{long_str}")),
            ),
            (
                &long_dir_file_str,
                PathBuf::from(format!("\\\\?\\{temp_dir_path_str}\\{long_str}\\file.txt")),
            ),
        ];

        for (input, expected) in test_cases {
            let result = canonical_path(input);
            assert_eq!(
                result,
                expected,
                "Expected canonical path for {} to be {}, but got {}",
                input,
                expected.to_string_lossy(),
                result.to_string_lossy()
            );
        }
    }

    #[cfg(not(windows))]
    #[ignore]
    #[test]
    fn test_canonical_path_unix() {
        let cur_dir = std::env::current_dir().unwrap();

        let test_cases = vec![
            (r"/home/.././etc/./../usr/bin", PathBuf::from(r"/usr/bin")),
            (
                r"/this_folder_does_not_exist/run/.././run/docker.sock",
                PathBuf::from(r"/this_folder_does_not_exist/run/docker.sock"),
            ),
            (r"/../../var", PathBuf::from(r"/var")),
            (r"/../../var_n/.", PathBuf::from(r"/var_n")),
            (
                r"///var_n//foo_n/foo_n//./././../bar_n/",
                PathBuf::from(r"/var_n/foo_n/bar_n/"),
            ),
            (r".", cur_dir.clone()),
            (r".//some_not_existing_folder/..", cur_dir.clone()),
            (r"./some_not_existing_folder///..//", cur_dir.join("")),
            (r"foo_n////var_n", cur_dir.join("foo_n").join("var_n")),
            (r"foo_n/../var_n/../cat_n/", cur_dir.join("cat_n")),
            (r"./foo_n/././..", cur_dir.clone()),
        ];

        for (input, expected) in test_cases {
            let result = canonical_path(input);
            assert_eq!(
                result,
                expected,
                "Expected canonical path for {} to be {}, but got {}",
                input,
                expected.to_string_lossy(),
                result.to_string_lossy()
            );
        }
    }
}
