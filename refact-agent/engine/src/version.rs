use shadow_rs::shadow;

shadow!(build);

#[cfg(test)]
mod tests {
    use super::build;

    #[test]
    fn build_package_version_and_commit_hash_are_valid() {
        assert_eq!(build::PKG_VERSION, env!("CARGO_PKG_VERSION"));

        let commit_hash = build::COMMIT_HASH;
        println!("COMMIT_HASH={commit_hash}");
        assert!(
            commit_hash.is_empty()
                || (commit_hash.len() == 40
                    && commit_hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        );
    }
}
