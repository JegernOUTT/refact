pub mod automation;
pub mod cron_expr;
pub mod jitter;
pub mod retry;
pub mod schedule;
pub mod sources;
pub mod store;
pub mod types;

#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) fn tempdir() -> tempfile::TempDir {
        let base = std::env::temp_dir();
        #[cfg(unix)]
        let base = base.canonicalize().expect("canonical temporary directory");
        tempfile::tempdir_in(base).expect("temporary test directory")
    }
}
