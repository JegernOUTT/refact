use crate::skills::SkillIndex;

pub fn skill_relevance_score(skill_description: &str, user_message: &str) -> f32 {
    fn tokenize(s: &str) -> std::collections::HashSet<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t: &&str| t.len() >= 3)
            .map(|t| t.to_string())
            .collect()
    }
    let desc_tokens = tokenize(skill_description);
    let msg_tokens = tokenize(user_message);
    if desc_tokens.is_empty() || msg_tokens.is_empty() {
        return 0.0;
    }
    let intersection = desc_tokens.intersection(&msg_tokens).count();
    let union_count = desc_tokens.len() + msg_tokens.len() - intersection;
    if union_count == 0 {
        return 0.0;
    }
    intersection as f32 / union_count as f32
}

pub fn select_relevant_skills(
    skills: &[SkillIndex],
    user_message: &str,
    max_skills: usize,
    threshold: f32,
) -> Vec<String> {
    let mut scored: Vec<(f32, String)> = skills
        .iter()
        .filter(|s| s.user_invocable)
        .filter_map(|s| {
            let score = skill_relevance_score(&s.description, user_message);
            if score >= threshold {
                Some((score, s.name.clone()))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(max_skills);
    scored.into_iter().map(|(_, name)| name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_dirs::CommandSource;

    fn make_skill_index(name: &str, description: &str) -> SkillIndex {
        SkillIndex {
            name: name.to_string(),
            description: description.to_string(),
            user_invocable: true,
            source: CommandSource::GlobalRefact,
        }
    }

    #[test]
    fn test_skills_select_matching() {
        let skills = vec![
            make_skill_index(
                "security-review",
                "reviews code for security vulnerabilities",
            ),
            make_skill_index("code-explainer", "explains code using analogies"),
        ];
        let selected = select_relevant_skills(&skills, "security vulnerability review", 2, 0.1);
        assert!(selected.contains(&"security-review".to_string()));
    }

    #[test]
    fn test_skills_select_no_match_above_threshold() {
        let skills = vec![make_skill_index(
            "security-review",
            "reviews security vulnerabilities",
        )];
        let selected = select_relevant_skills(&skills, "breakfast cereal", 2, 0.5);
        assert!(selected.is_empty());
    }

    #[test]
    fn test_skills_select_respects_max_skills() {
        let skills = vec![
            make_skill_index("skill1", "rust code review analysis checks"),
            make_skill_index("skill2", "rust code analysis review checks"),
            make_skill_index("skill3", "rust review code analysis checks"),
        ];
        let selected = select_relevant_skills(&skills, "rust code review analysis", 2, 0.1);
        assert!(selected.len() <= 2, "Should not exceed max_skills");
    }

    #[test]
    fn test_skills_select_returns_highest_scoring() {
        let skills = vec![
            make_skill_index("low-scorer", "completely unrelated topic"),
            make_skill_index("high-scorer", "security review vulnerabilities code"),
        ];
        let selected = select_relevant_skills(&skills, "security review vulnerabilities", 1, 0.1);
        assert_eq!(selected, vec!["high-scorer"]);
    }

    #[test]
    fn test_skills_select_threshold_filters() {
        let skills = vec![
            make_skill_index("low-match", "code quality"),
            make_skill_index("high-match", "security review vulnerabilities audit"),
        ];
        let low_threshold = select_relevant_skills(&skills, "security review", 2, 0.01);
        let high_threshold = select_relevant_skills(&skills, "security review", 2, 0.5);
        assert!(
            low_threshold.len() >= high_threshold.len(),
            "Lower threshold should match more"
        );
    }

    #[test]
    fn test_skills_select_user_invocable_false_prevents_auto_trigger() {
        let skills = vec![SkillIndex {
            name: "security-review".to_string(),
            description: "reviews security vulnerabilities code".to_string(),
            user_invocable: false,
            source: CommandSource::GlobalRefact,
        }];
        let selected = select_relevant_skills(&skills, "security review vulnerabilities", 2, 0.1);
        assert!(
            selected.is_empty(),
            "user_invocable=false should prevent auto-trigger"
        );
    }
}
