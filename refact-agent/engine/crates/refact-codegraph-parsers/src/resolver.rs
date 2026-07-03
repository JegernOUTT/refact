use std::collections::{BTreeSet, HashMap, HashSet};

pub const CONFIDENCE_EXACT: f32 = 1.0;
pub const CONFIDENCE_ALIAS: f32 = 0.8;
pub const CONFIDENCE_FUZZY_UNIQUE: f32 = 0.5;
pub const CONFIDENCE_FUZZY_AMBIGUOUS: f32 = 0.3;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ResolutionTier {
    Exact,
    Alias,
    Fuzzy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    pub target: String,
    pub confidence: f32,
    pub tier: ResolutionTier,
}

#[derive(Default)]
pub struct Resolver {
    full_paths: HashSet<String>,
    by_last_segment: HashMap<String, BTreeSet<String>>,
    aliases: HashMap<String, String>,
}

fn split_segments(name: &str) -> Vec<&str> {
    name.split("::")
        .flat_map(|seg| seg.split('.'))
        .flat_map(|seg| seg.split('/'))
        .filter(|seg| !seg.is_empty())
        .collect()
}

fn last_segment(name: &str) -> Option<String> {
    split_segments(name).last().map(|s| s.to_string())
}

fn first_segment(name: &str) -> Option<String> {
    split_segments(name).first().map(|s| s.to_string())
}

impl Resolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_symbol(&mut self, full_path: &str) {
        if full_path.is_empty() {
            return;
        }
        self.full_paths.insert(full_path.to_string());
        if let Some(last) = last_segment(full_path) {
            self.by_last_segment
                .entry(last)
                .or_default()
                .insert(full_path.to_string());
        }
    }

    pub fn add_alias(&mut self, alias: &str, real: &str) {
        if alias.is_empty() || real.is_empty() {
            return;
        }
        self.aliases.insert(alias.to_string(), real.to_string());
    }

    pub fn resolve(&self, name: &str) -> Option<Resolution> {
        if name.is_empty() {
            return None;
        }

        if self.full_paths.contains(name) {
            return Some(Resolution {
                target: name.to_string(),
                confidence: CONFIDENCE_EXACT,
                tier: ResolutionTier::Exact,
            });
        }

        if let Some(res) = self.resolve_via_alias(name) {
            return Some(res);
        }

        self.resolve_fuzzy(name)
    }

    fn resolve_via_alias(&self, name: &str) -> Option<Resolution> {
        if let Some(real) = self.aliases.get(name) {
            if self.full_paths.contains(real) {
                return Some(Resolution {
                    target: real.clone(),
                    confidence: CONFIDENCE_ALIAS,
                    tier: ResolutionTier::Alias,
                });
            }
        }

        let first = first_segment(name)?;
        let real_prefix = self.aliases.get(&first)?;
        let rest: Vec<&str> = split_segments(name).into_iter().skip(1).collect();
        let rewritten = if rest.is_empty() {
            real_prefix.clone()
        } else {
            format!("{}::{}", real_prefix, rest.join("::"))
        };
        if self.full_paths.contains(&rewritten) {
            return Some(Resolution {
                target: rewritten,
                confidence: CONFIDENCE_ALIAS,
                tier: ResolutionTier::Alias,
            });
        }
        None
    }

    fn resolve_fuzzy(&self, name: &str) -> Option<Resolution> {
        let last = last_segment(name)?;
        let candidates = self.by_last_segment.get(&last)?;
        match candidates.len() {
            0 => None,
            1 => Some(Resolution {
                target: candidates.iter().next().unwrap().clone(),
                confidence: CONFIDENCE_FUZZY_UNIQUE,
                tier: ResolutionTier::Fuzzy,
            }),
            _ => Some(Resolution {
                target: candidates.iter().next().unwrap().clone(),
                confidence: CONFIDENCE_FUZZY_AMBIGUOUS,
                tier: ResolutionTier::Fuzzy,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver_with_symbols(symbols: &[&str]) -> Resolver {
        let mut r = Resolver::new();
        for s in symbols {
            r.add_symbol(s);
        }
        r
    }

    #[test]
    fn exact_path_resolves_with_full_confidence() {
        let r = resolver_with_symbols(&["app::services::user::create_user"]);
        let res = r.resolve("app::services::user::create_user").unwrap();
        assert_eq!(res.tier, ResolutionTier::Exact);
        assert_eq!(res.confidence, CONFIDENCE_EXACT);
        assert_eq!(res.target, "app::services::user::create_user");
    }

    #[test]
    fn import_alias_resolves_to_real_symbol() {
        let mut r = resolver_with_symbols(&["app::services::user::create_user"]);
        r.add_alias("createUser", "app::services::user::create_user");
        let res = r.resolve("createUser").unwrap();
        assert_eq!(res.tier, ResolutionTier::Alias);
        assert_eq!(res.confidence, CONFIDENCE_ALIAS);
        assert_eq!(res.target, "app::services::user::create_user");
    }

    #[test]
    fn barrel_alias_rewrites_first_segment_then_appends_rest() {
        let mut r = resolver_with_symbols(&["app::services::user::create_user"]);
        r.add_alias("userService", "app::services::user");
        let res = r.resolve("userService::create_user").unwrap();
        assert_eq!(res.tier, ResolutionTier::Alias);
        assert_eq!(res.confidence, CONFIDENCE_ALIAS);
        assert_eq!(res.target, "app::services::user::create_user");
    }

    #[test]
    fn namespace_alias_handles_dotted_reference() {
        let mut r = resolver_with_symbols(&["pkg::mod_a::Widget"]);
        r.add_alias("mod_a", "pkg::mod_a");
        let res = r.resolve("mod_a.Widget").unwrap();
        assert_eq!(res.tier, ResolutionTier::Alias);
        assert_eq!(res.target, "pkg::mod_a::Widget");
    }

    #[test]
    fn fuzzy_last_segment_resolves_unique_candidate() {
        let r = resolver_with_symbols(&["app::services::user::create_user"]);
        let res = r.resolve("create_user").unwrap();
        assert_eq!(res.tier, ResolutionTier::Fuzzy);
        assert_eq!(res.confidence, CONFIDENCE_FUZZY_UNIQUE);
        assert_eq!(res.target, "app::services::user::create_user");
    }

    #[test]
    fn fuzzy_ambiguous_last_segment_lowers_confidence() {
        let r = resolver_with_symbols(&["a::handler::run", "b::worker::run"]);
        let res = r.resolve("run").unwrap();
        assert_eq!(res.tier, ResolutionTier::Fuzzy);
        assert_eq!(res.confidence, CONFIDENCE_FUZZY_AMBIGUOUS);
        assert!(res.target == "a::handler::run" || res.target == "b::worker::run");
    }

    #[test]
    fn unknown_reference_does_not_resolve() {
        let r = resolver_with_symbols(&["a::b::c"]);
        assert!(r.resolve("does_not_exist").is_none());
    }

    #[test]
    fn alias_preferred_over_fuzzy_when_both_apply() {
        let mut r = resolver_with_symbols(&["real::create", "other::create"]);
        r.add_alias("create", "real::create");
        let res = r.resolve("create").unwrap();
        assert_eq!(res.tier, ResolutionTier::Alias);
        assert_eq!(res.target, "real::create");
    }
}
