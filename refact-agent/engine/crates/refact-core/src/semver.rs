use std::cmp::Ordering;

pub fn compare_versions(left: &str, right: &str) -> Ordering {
    match (parse_semver(left), parse_semver(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemverParts {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

impl Ord for SemverParts {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| compare_prerelease(self.pre.as_deref(), other.pre.as_deref()))
    }
}

impl PartialOrd for SemverParts {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_semver(version: &str) -> Option<SemverParts> {
    let without_prefix = version.trim().strip_prefix('v').unwrap_or(version.trim());
    let without_build = without_prefix
        .split_once('+')
        .map(|(base, _)| base)
        .unwrap_or(without_prefix);
    let (core, pre) = without_build
        .split_once('-')
        .map(|(core, pre)| (core, Some(pre)))
        .unwrap_or((without_build, None));
    let parts = core.split('.').collect::<Vec<_>>();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let parse_part = |part: &str| {
        if part.chars().all(|ch| ch.is_ascii_digit()) {
            part.parse::<u64>().ok()
        } else {
            None
        }
    };
    let pre = match pre {
        Some("") => return None,
        Some(pre) => Some(pre.to_string()),
        None => None,
    };
    Some(SemverParts {
        major: parse_part(parts[0])?,
        minor: parse_part(parts[1])?,
        patch: parse_part(parts[2])?,
        pre,
    })
}

fn compare_prerelease(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => compare_prerelease_identifiers(left, right),
    }
}

fn compare_prerelease_identifiers(left: &str, right: &str) -> Ordering {
    let mut left_parts = left.split('.');
    let mut right_parts = right.split('.');
    loop {
        match (left_parts.next(), right_parts.next()) {
            (Some(left), Some(right)) => {
                let ordering = compare_prerelease_identifier(left, right);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}

fn compare_prerelease_identifier(left: &str, right: &str) -> Ordering {
    let left_numeric = parse_numeric_identifier(left);
    let right_numeric = parse_numeric_identifier(right);
    match (left_numeric, right_numeric) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.cmp(right),
    }
}

fn parse_numeric_identifier(value: &str) -> Option<u64> {
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_compare_orders_numeric_components() {
        assert_eq!(compare_versions("1.2.0", "1.10.0"), Ordering::Less);
        assert_eq!(compare_versions("2.0.0", "1.10.0"), Ordering::Greater);
    }

    #[test]
    fn semver_compare_orders_prerelease_before_release() {
        assert_eq!(compare_versions("1.0.0-rc1", "1.0.0"), Ordering::Less);
        assert_eq!(
            compare_versions("1.0.0-alpha.9", "1.0.0-alpha.10"),
            Ordering::Less
        );
    }
}
