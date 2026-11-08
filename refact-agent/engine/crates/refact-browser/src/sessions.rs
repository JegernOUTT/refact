//! Validation shared by the embedding application's named browser sessions.

/// Accept only bounded, non-path session identifiers, reserving the default name.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name != "shared_default"
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
