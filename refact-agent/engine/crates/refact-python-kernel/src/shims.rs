//! Pure-Python stand-ins for stdlib modules Monty does not bundle, served for `import`
//! ahead of project files.
use monty_types::ModuleFile;

const SHIMS: &[(&str, &str)] = &[("io", include_str!("shims/io.py"))];

pub const SHIM_ROOT: &str = "/refact-shims";

pub fn module_file(module: &str) -> Option<ModuleFile> {
    let (name, source) = SHIMS.iter().find(|(name, _)| *name == module)?;
    Some(ModuleFile {
        path: format!("{SHIM_ROOT}/{name}.py"),
        source: (*source).to_owned(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn shims_never_stand_in_for_a_bundled_module() {
        let bundled = monty::builtin_module_names();
        for (name, _) in super::SHIMS {
            assert!(!bundled.contains(name), "{name} is bundled");
        }
    }
}
