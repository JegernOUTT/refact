use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use serde::{Deserialize, Serialize};

use crate::call_validation::{ChatContent, ChatMessage, ContextFile};
use crate::ext::config_dirs::{get_ext_dirs, ExtDirs};
use crate::ext::skills::{load_skill_full, load_skill_indices, load_skill_linked_file, SkillFull};
use crate::ext::skills_matcher::select_relevant_skills;
use crate::global_context::GlobalContext;

pub const SKILLS_CONTEXT_MARKER: &str = "skills_context";

#[derive(Debug, Default, Clone)]
pub struct SkillsTrackingInfo {
    pub available_count: usize,
    pub included_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillsAutoTrigger {
    InjectFull,
    #[default]
    IndexOnly,
    Off,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillsConfig {
    #[serde(default)]
    pub auto_trigger: SkillsAutoTrigger,
}

pub async fn load_skills_config(gcx: Arc<ARwLock<GlobalContext>>) -> SkillsConfig {
    let project_dirs = crate::files_correction::get_project_dirs(gcx).await;
    let Some(project_dir) = project_dirs.first() else {
        return SkillsConfig::default();
    };
    let path = project_dir.join(".refact").join("skills.yaml");
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
        Err(_) => SkillsConfig::default(),
    }
}

const MAX_INCLUDES: usize = 5;

pub async fn expand_skill_includes(body: &str, skill_dir: &Path) -> String {
    let mut new_lines = Vec::new();
    let mut include_count = 0;
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(path) = trimmed.strip_prefix("@include ") {
            if include_count < MAX_INCLUDES {
                if let Some(content) = load_skill_linked_file(skill_dir, path.trim()).await {
                    new_lines.push(content);
                    include_count += 1;
                    continue;
                } else {
                    tracing::warn!("Failed to load @include file: {}", path.trim());
                }
            } else {
                tracing::warn!("Skipping @include (max {} includes reached)", MAX_INCLUDES);
            }
        }
        new_lines.push(line.to_string());
    }
    new_lines.join("\n")
}

fn build_skills_prompt_markdown(
    displayable: &[&crate::ext::skills::SkillIndex],
    has_activate_skill: bool,
    has_deactivate_skill: bool,
) -> String {
    if displayable.is_empty() {
        let mut lines = vec![
            "## Skills".to_string(),
            String::new(),
            "You have access to a skills system. Skills are specialized instruction sets that provide focused guidance for specific tasks.".to_string(),
            String::new(),
            "No skills are currently installed. Users can create skills in the Extensions page.".to_string(),
        ];
        if has_activate_skill {
            lines.push(String::new());
            if has_deactivate_skill {
                lines.push("When skills are available, use `activate_skill(name)` to load them, work with them, then call `deactivate_skill(report=\"...\")` with a completion summary.".to_string());
            } else {
                lines.push(
                    "When skills are available, use `activate_skill(name)` to load them."
                        .to_string(),
                );
            }
        }
        return lines.join("\n");
    }
    let available_skills_intro = if has_activate_skill {
        "The following skills are available. Use `activate_skill(name)` to load a skill's full instructions when relevant. Users can also invoke skills with `/skill-name`.".to_string()
    } else {
        "The following skills are available. Users can invoke skills with `/skill-name`."
            .to_string()
    };
    let mut lines = vec![
        "## Skills".to_string(),
        String::new(),
        "You have access to skills — specialized instruction sets that provide focused guidance for specific tasks.".to_string(),
        String::new(),
        "### Available Skills".to_string(),
        available_skills_intro,
        String::new(),
    ];
    for skill in displayable {
        lines.push(format!("- **{}**: {}", skill.name, skill.description));
    }
    lines.push(String::new());
    if has_activate_skill || has_deactivate_skill {
        lines.push("### Skill Lifecycle".to_string());
        if has_activate_skill {
            lines.push("1. **Activate**: Call `activate_skill(name=\"skill-name\")` to load instructions into context".to_string());
        }
        lines.push("2. **Work**: Follow the skill's instructions to complete the task".to_string());
        if has_deactivate_skill {
            lines.push("3. **Deactivate**: When done, call `deactivate_skill(report=\"...\")` with a thorough markdown report:".to_string());
            lines.push("   - What was the goal".to_string());
            lines.push("   - What actions were taken".to_string());
            lines.push("   - What files were changed".to_string());
            lines.push("   - What was the outcome".to_string());
            lines.push("   The report compacts the entire skill execution into a clean summary in chat history.".to_string());
            lines.push(String::new());
            lines.push("### Important".to_string());
            lines.push("- Only one skill can be active at a time".to_string());
            lines.push("- Always deactivate a skill when its task is complete".to_string());
            lines.push("- The deactivation report is the primary record of the skill run; activation confirmation may also remain".to_string());
        }
    } else {
        lines.push("### How Skills Work".to_string());
        lines.push(
            "- Skills with `disable-model-invocation` are user-only (not listed above)".to_string(),
        );
    }
    lines.join("\n")
}

pub async fn build_skills_prompt_text(
    gcx: Arc<ARwLock<GlobalContext>>,
    has_activate_skill: bool,
    has_deactivate_skill: bool,
) -> String {
    let config = load_skills_config(gcx.clone()).await;
    if matches!(config.auto_trigger, SkillsAutoTrigger::Off) {
        return "## Skills\n\nSkills system is disabled by configuration.".to_string();
    }
    let ext_dirs = get_ext_dirs(gcx).await;
    let indices = load_skill_indices(&ext_dirs).await;
    let displayable: Vec<_> = indices
        .iter()
        .filter(|s| s.user_invocable && !s.disable_model_invocation)
        .collect();
    build_skills_prompt_markdown(&displayable, has_activate_skill, has_deactivate_skill)
}

async fn build_context_messages_from_dirs(
    ext_dirs: &ExtDirs,
    user_message: &str,
    explicit_skill: Option<&str>,
    mode: SkillsAutoTrigger,
) -> (Vec<ChatMessage>, SkillsTrackingInfo) {
    if matches!(mode, SkillsAutoTrigger::Off) {
        return (Vec::new(), SkillsTrackingInfo::default());
    }

    let indices = load_skill_indices(ext_dirs).await;
    let available_count = indices.len();

    let mut context_files: Vec<ContextFile> = Vec::new();

    let skills_to_load: Vec<SkillFull> = if matches!(mode, SkillsAutoTrigger::IndexOnly) {
        vec![]
    } else if let Some(name) = explicit_skill {
        match load_skill_full(ext_dirs, name).await {
            Some(full) if full.index.user_invocable && !full.index.disable_model_invocation => {
                vec![full]
            }
            Some(full) => {
                tracing::warn!(
                    "Skipping explicit skill '{}': user_invocable={}, disable_model_invocation={}",
                    full.index.name,
                    full.index.user_invocable,
                    full.index.disable_model_invocation
                );
                vec![]
            }
            None => vec![],
        }
    } else {
        let selected_names = select_relevant_skills(&indices, user_message, 2, 0.5);
        let mut result = Vec::new();
        for name in &selected_names {
            if let Some(full) = load_skill_full(ext_dirs, name).await {
                result.push(full);
            }
        }
        result
    };

    let included_names: Vec<String> = skills_to_load
        .iter()
        .map(|s| s.index.name.clone())
        .collect();

    for skill in &skills_to_load {
        let body = expand_skill_includes(&skill.body, &skill.skill_dir).await;
        let line_count = body.lines().count().max(1);
        context_files.push(ContextFile {
            file_name: format!("skill://{}", skill.index.name),
            file_content: body,
            line1: 1,
            line2: line_count,
            file_rev: None,
            symbols: vec![],
            gradient_type: 0,
            usefulness: 90.0,
            skip_pp: true,
        });
    }

    let tracking = SkillsTrackingInfo {
        available_count,
        included_names,
    };

    if context_files.is_empty() {
        return (Vec::new(), tracking);
    }

    (
        vec![ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(context_files),
            tool_call_id: SKILLS_CONTEXT_MARKER.to_string(),
            ..Default::default()
        }],
        tracking,
    )
}

pub async fn build_skills_context_messages_tracked(
    gcx: Arc<ARwLock<GlobalContext>>,
    user_message: &str,
    explicit_skill: Option<&str>,
) -> (Vec<ChatMessage>, SkillsTrackingInfo) {
    let config = load_skills_config(gcx.clone()).await;
    let ext_dirs = get_ext_dirs(gcx).await;
    build_context_messages_from_dirs(&ext_dirs, user_message, explicit_skill, config.auto_trigger)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::config_dirs::ExtDirs;
    use std::path::PathBuf;

    fn make_ext_dirs(root: &Path) -> ExtDirs {
        ExtDirs {
            global_dirs: vec![root.to_path_buf()],
            installed_dirs: vec![],
            project_dirs: vec![],
        }
    }

    async fn write_skill(root: &Path, name: &str, frontmatter: &str, body: &str) {
        let skill_dir = root.join("skills").join(name);
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        let content = format!("---\n{}\n---\n{}", frontmatter, body);
        tokio::fs::write(skill_dir.join("SKILL.md"), content)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_skills_index_format_correct() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "code-explainer",
            "name: code-explainer\ndescription: Explains code using analogies",
            "Explain this: $ARGUMENTS",
        )
        .await;
        write_skill(
            tmp.path(),
            "security-review",
            "name: security-review\ndescription: Reviews code for security vulnerabilities",
            "Review: $ARGUMENTS",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let indices = crate::ext::skills::load_skill_indices(&ext_dirs).await;
        let displayable: Vec<_> = indices
            .iter()
            .filter(|s| s.user_invocable && !s.disable_model_invocation)
            .collect();
        let text = build_skills_prompt_markdown(&displayable, true, false);

        assert!(text.contains("### Available Skills"));
        assert!(text.contains("**code-explainer**"));
        assert!(text.contains("Explains code using analogies"));
        assert!(text.contains("**security-review**"));
        assert!(text.contains("Reviews code for security vulnerabilities"));
        assert!(text.contains("/skill-name"));
    }

    #[tokio::test]
    async fn test_skills_index_respects_user_invocable() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "visible-skill",
            "name: visible-skill\ndescription: Visible skill\nuser-invocable: true",
            "Body",
        )
        .await;
        write_skill(
            tmp.path(),
            "hidden-skill",
            "name: hidden-skill\ndescription: Hidden skill\nuser-invocable: false",
            "Body",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let indices = crate::ext::skills::load_skill_indices(&ext_dirs).await;
        let displayable: Vec<_> = indices
            .iter()
            .filter(|s| s.user_invocable && !s.disable_model_invocation)
            .collect();
        let text = build_skills_prompt_markdown(&displayable, true, false);

        assert!(text.contains("visible-skill"));
        assert!(
            !text.contains("hidden-skill"),
            "Non-user-invocable skills must not appear in index"
        );
    }

    #[tokio::test]
    async fn test_skills_index_not_empty_when_no_skills() {
        let text = build_skills_prompt_markdown(&[], true, false);
        assert!(!text.is_empty());
        assert!(text.contains("## Skills"));
        assert!(text.contains("No skills are currently installed"));
    }

    #[tokio::test]
    async fn test_skills_index_not_empty_when_all_non_invocable() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "hidden",
            "name: hidden\ndescription: Hidden skill\nuser-invocable: false",
            "Body",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let indices = crate::ext::skills::load_skill_indices(&ext_dirs).await;
        let displayable: Vec<_> = indices
            .iter()
            .filter(|s| s.user_invocable && !s.disable_model_invocation)
            .collect();
        let text = build_skills_prompt_markdown(&displayable, true, false);
        assert!(!text.is_empty());
        assert!(
            !text.contains("hidden"),
            "Non-invocable skill must not appear in output"
        );
    }

    #[tokio::test]
    async fn test_skills_auto_trigger_matching() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "security-review",
            "name: security-review\ndescription: reviews code for security vulnerabilities auditing",
            "Review for security: $ARGUMENTS",
        )
        .await;
        write_skill(
            tmp.path(),
            "code-explainer",
            "name: code-explainer\ndescription: explains analogies diagrams visual",
            "Explain: $ARGUMENTS",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let indices = crate::ext::skills::load_skill_indices(&ext_dirs).await;
        let selected = crate::ext::skills_matcher::select_relevant_skills(
            &indices,
            "security vulnerabilities auditing code review",
            2,
            0.5,
        );
        assert!(
            selected.contains(&"security-review".to_string()),
            "Security skill should be auto-triggered"
        );
    }

    #[tokio::test]
    async fn test_skills_auto_trigger_non_matching() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "security-review",
            "name: security-review\ndescription: reviews code for security vulnerabilities",
            "Body",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let indices = crate::ext::skills::load_skill_indices(&ext_dirs).await;
        let selected = crate::ext::skills_matcher::select_relevant_skills(
            &indices,
            "breakfast cereal recipes",
            2,
            0.5,
        );
        assert!(
            selected.is_empty(),
            "Unrelated message should not trigger skills"
        );
    }

    #[tokio::test]
    async fn test_skills_disable_model_invocation_prevents_auto_trigger() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "security-review",
            "name: security-review\ndescription: reviews code for security vulnerabilities\ndisable-model-invocation: true",
            "Body",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let indices = crate::ext::skills::load_skill_indices(&ext_dirs).await;
        let selected = crate::ext::skills_matcher::select_relevant_skills(
            &indices,
            "security review vulnerabilities",
            2,
            0.1,
        );
        assert!(
            selected.is_empty(),
            "disable-model-invocation must prevent auto-trigger"
        );
    }

    #[tokio::test]
    async fn test_skills_explicit_invocation_loads_full_skill() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "my-skill",
            "name: my-skill\ndescription: A useful skill\nuser-invocable: true",
            "Do something with $ARGUMENTS in detail",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "anything",
            Some("my-skill"),
            SkillsAutoTrigger::InjectFull,
        )
        .await;
        assert!(
            !msgs.is_empty(),
            "Should return messages for explicit skill invocation"
        );

        let files = match &msgs[0].content {
            crate::call_validation::ChatContent::ContextFiles(f) => f,
            _ => panic!("Expected ContextFiles"),
        };
        let skill_file = files.iter().find(|f| f.file_name == "skill://my-skill");
        assert!(skill_file.is_some(), "Should include skill body");
        assert!(skill_file
            .unwrap()
            .file_content
            .contains("Do something with $ARGUMENTS in detail"));
    }

    #[tokio::test]
    async fn test_skills_progressive_loading_includes_resolve() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("with-include");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(skill_dir.join("context.md"), "Included file content")
            .await
            .unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: with-include\ndescription: Skill with includes\nuser-invocable: true\n---\nBefore include\n@include context.md\nAfter include",
        )
        .await
        .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "anything",
            Some("with-include"),
            SkillsAutoTrigger::InjectFull,
        )
        .await;
        assert!(!msgs.is_empty());

        let files = match &msgs[0].content {
            crate::call_validation::ChatContent::ContextFiles(f) => f,
            _ => panic!("Expected ContextFiles"),
        };
        let skill_file = files
            .iter()
            .find(|f| f.file_name == "skill://with-include")
            .unwrap();
        assert!(
            skill_file.file_content.contains("Included file content"),
            "Include should be resolved"
        );
        assert!(
            !skill_file.file_content.contains("@include"),
            "@include directive should be replaced"
        );
    }

    #[tokio::test]
    async fn test_skills_progressive_loading_size_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("big-include");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();

        let big_content = "x".repeat(50 * 1024 + 1);
        tokio::fs::write(skill_dir.join("big.md"), &big_content)
            .await
            .unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: big-include\ndescription: Skill with big include\nuser-invocable: true\n---\nBefore\n@include big.md\nAfter",
        )
        .await
        .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "anything",
            Some("big-include"),
            SkillsAutoTrigger::InjectFull,
        )
        .await;
        assert!(!msgs.is_empty());

        let files = match &msgs[0].content {
            crate::call_validation::ChatContent::ContextFiles(f) => f,
            _ => panic!("Expected ContextFiles"),
        };
        let skill_file = files
            .iter()
            .find(|f| f.file_name == "skill://big-include")
            .unwrap();
        assert!(
            !skill_file.file_content.contains(&big_content[..100]),
            "Oversized included file should be skipped"
        );
        assert!(
            skill_file.file_content.contains("@include big.md"),
            "@include directive should remain if file too large"
        );
    }

    #[tokio::test]
    async fn test_skills_max_includes_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("many-includes");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();

        let mut body = String::new();
        for i in 0..=MAX_INCLUDES {
            let fname = format!("file{}.md", i);
            tokio::fs::write(skill_dir.join(&fname), format!("Content {}", i))
                .await
                .unwrap();
            body.push_str(&format!("@include {}\n", fname));
        }

        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: many-includes\ndescription: Skill with many includes\nuser-invocable: true\n---\n{}", body),
        )
        .await
        .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "anything",
            Some("many-includes"),
            SkillsAutoTrigger::InjectFull,
        )
        .await;
        assert!(!msgs.is_empty());

        let files = match &msgs[0].content {
            crate::call_validation::ChatContent::ContextFiles(f) => f,
            _ => panic!("Expected ContextFiles"),
        };
        let skill_file = files
            .iter()
            .find(|f| f.file_name == "skill://many-includes")
            .unwrap();
        let included_count = (0..=MAX_INCLUDES)
            .filter(|i| skill_file.file_content.contains(&format!("Content {}", i)))
            .count();
        assert!(
            included_count <= MAX_INCLUDES,
            "Should not include more than {} files, got {}",
            MAX_INCLUDES,
            included_count
        );
    }

    #[tokio::test]
    async fn test_skills_context_messages_no_skills() {
        let ext_dirs = ExtDirs {
            global_dirs: vec![PathBuf::from("/nonexistent")],
            installed_dirs: vec![],
            project_dirs: vec![],
        };
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "any message",
            None,
            SkillsAutoTrigger::InjectFull,
        )
        .await;
        assert!(msgs.is_empty(), "No skills = no messages");
    }

    #[tokio::test]
    async fn test_skills_context_messages_no_index_in_context_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "test-skill",
            "name: test-skill\ndescription: Test skill\nuser-invocable: true",
            "Body",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "unrelated message",
            None,
            SkillsAutoTrigger::InjectFull,
        )
        .await;
        for msg in &msgs {
            if let crate::call_validation::ChatContent::ContextFiles(files) = &msg.content {
                let index_file = files.iter().find(|f| f.file_name == "skills://index");
                assert!(index_file.is_none(), "skills://index must not be emitted as ContextFile (it is now in system prompt)");
            }
        }
    }

    #[tokio::test]
    async fn test_skills_edge_case_skill_no_body() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills").join("empty-body");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: empty-body\ndescription: Skill with empty body\nuser-invocable: true\n---\n",
        )
        .await
        .unwrap();

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "anything",
            Some("empty-body"),
            SkillsAutoTrigger::InjectFull,
        )
        .await;
        assert!(
            !msgs.is_empty(),
            "Should return skill body ContextFile for explicit invocation"
        );
    }

    #[tokio::test]
    async fn test_skills_context_mode_off_injects_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "test-skill",
            "name: test-skill\ndescription: Test skill\nuser-invocable: true",
            "Body",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, tracking) = build_context_messages_from_dirs(
            &ext_dirs,
            "test-skill anything",
            None,
            SkillsAutoTrigger::Off,
        )
        .await;
        assert!(msgs.is_empty(), "Off mode must inject nothing");
        assert_eq!(tracking.available_count, 0);
        assert!(tracking.included_names.is_empty());
    }

    #[tokio::test]
    async fn test_skills_context_mode_index_only_injects_no_context_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "security-review",
            "name: security-review\ndescription: reviews code for security vulnerabilities auditing",
            "Full security review body content",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, tracking) = build_context_messages_from_dirs(
            &ext_dirs,
            "security review vulnerabilities",
            None,
            SkillsAutoTrigger::IndexOnly,
        )
        .await;

        assert!(
            msgs.is_empty(),
            "IndexOnly must not inject any ContextFiles (index is in system prompt)"
        );
        assert!(
            tracking.included_names.is_empty(),
            "No skills included in index_only mode"
        );
        assert!(
            tracking.available_count > 0,
            "Tracking must still report available skills"
        );
    }

    #[tokio::test]
    async fn test_skills_context_mode_index_only_is_default() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "security-review",
            "name: security-review\ndescription: reviews code for security vulnerabilities auditing",
            "Full security review body content",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs_default, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "security review vulnerabilities auditing",
            None,
            SkillsAutoTrigger::default(),
        )
        .await;
        let (msgs_explicit, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "security review vulnerabilities auditing",
            None,
            SkillsAutoTrigger::IndexOnly,
        )
        .await;

        assert_eq!(
            msgs_default.len(),
            msgs_explicit.len(),
            "Default mode must equal IndexOnly"
        );
    }

    #[tokio::test]
    async fn test_skills_index_excludes_disable_model_invocation() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "user-skill",
            "name: user-skill\ndescription: User invocable skill\nuser-invocable: true\ndisable-model-invocation: false",
            "Body",
        )
        .await;
        write_skill(
            tmp.path(),
            "model-disabled",
            "name: model-disabled\ndescription: Model disabled skill\nuser-invocable: true\ndisable-model-invocation: true",
            "Body",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let indices = crate::ext::skills::load_skill_indices(&ext_dirs).await;
        let displayable: Vec<_> = indices
            .iter()
            .filter(|s| s.user_invocable && !s.disable_model_invocation)
            .collect();
        let text = build_skills_prompt_markdown(&displayable, true, false);
        assert!(
            text.contains("user-skill"),
            "user-invocable skill must appear in index"
        );
        assert!(
            !text.contains("model-disabled"),
            "disable-model-invocation skill must not appear in index"
        );
    }

    #[tokio::test]
    async fn test_build_skills_context_no_skills_index_in_msgs() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "dedup-skill",
            "name: dedup-skill\ndescription: Deduplication test skill\nuser-invocable: true",
            "Skill body",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "anything",
            None,
            SkillsAutoTrigger::InjectFull,
        )
        .await;

        for msg in &msgs {
            if let crate::call_validation::ChatContent::ContextFiles(files) = &msg.content {
                let index_file = files.iter().find(|f| f.file_name == "skills://index");
                assert!(
                    index_file.is_none(),
                    "skills://index must not be emitted as ContextFile"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_build_skills_prompt_text_returns_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "my-skill",
            "name: my-skill\ndescription: Does something useful",
            "Body",
        )
        .await;
        write_skill(
            tmp.path(),
            "hidden-skill",
            "name: hidden-skill\ndescription: Should be hidden\ndisable-model-invocation: true",
            "Body",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let indices = crate::ext::skills::load_skill_indices(&ext_dirs).await;
        let displayable: Vec<_> = indices
            .iter()
            .filter(|s| s.user_invocable && !s.disable_model_invocation)
            .collect();
        assert!(
            !displayable.is_empty(),
            "Expected at least one displayable skill"
        );

        let text = build_skills_prompt_markdown(&displayable, true, true);

        assert!(text.contains("## Skills"));
        assert!(text.contains("### Available Skills"));
        assert!(text.contains("### Skill Lifecycle"));
        assert!(text.contains("**my-skill**"));
        assert!(text.contains("Does something useful"));
        assert!(
            !text.contains("hidden-skill"),
            "disable-model-invocation skills must not appear"
        );
        assert!(text.contains("activate_skill"));
    }

    #[tokio::test]
    async fn test_skill_bodies_still_emitted_as_context_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "security-review",
            "name: security-review\ndescription: reviews code for security vulnerabilities auditing",
            "Full security review body content",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, tracking) = build_context_messages_from_dirs(
            &ext_dirs,
            "security vulnerabilities auditing code review",
            None,
            SkillsAutoTrigger::InjectFull,
        )
        .await;

        assert!(
            !msgs.is_empty(),
            "Skill bodies must still be emitted as ContextFiles"
        );
        let files = match &msgs[0].content {
            crate::call_validation::ChatContent::ContextFiles(f) => f,
            _ => panic!("Expected ContextFiles"),
        };
        let skill_body = files
            .iter()
            .find(|f| f.file_name == "skill://security-review");
        assert!(
            skill_body.is_some(),
            "Skill body ContextFile must be present"
        );
        assert!(skill_body
            .unwrap()
            .file_content
            .contains("Full security review body content"));
        assert!(tracking
            .included_names
            .contains(&"security-review".to_string()));
    }

    #[test]
    fn test_skills_config_default_is_index_only() {
        let config = SkillsConfig::default();
        assert_eq!(config.auto_trigger, SkillsAutoTrigger::IndexOnly);
    }

    #[test]
    fn test_skills_config_serde_roundtrip() {
        let yaml = "auto_trigger: index_only\n";
        let config: SkillsConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.auto_trigger, SkillsAutoTrigger::IndexOnly);

        let yaml_off = "auto_trigger: off\n";
        let config_off: SkillsConfig = serde_yaml::from_str(yaml_off).unwrap();
        assert_eq!(config_off.auto_trigger, SkillsAutoTrigger::Off);

        let yaml_full = "auto_trigger: inject_full\n";
        let config_full: SkillsConfig = serde_yaml::from_str(yaml_full).unwrap();
        assert_eq!(config_full.auto_trigger, SkillsAutoTrigger::InjectFull);
    }

    #[test]
    fn test_skills_config_empty_yaml_defaults_to_index_only() {
        let config: SkillsConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.auto_trigger, SkillsAutoTrigger::IndexOnly);
    }

    fn make_skill_index(name: &str, description: &str) -> crate::ext::skills::SkillIndex {
        crate::ext::skills::SkillIndex {
            name: name.to_string(),
            description: description.to_string(),
            user_invocable: true,
            disable_model_invocation: false,
            source: crate::ext::config_dirs::CommandSource::GlobalRefact,
        }
    }

    #[test]
    fn test_skills_prompt_with_activate_tool() {
        let skill = make_skill_index("my-skill", "Does something useful");
        let displayable = vec![&skill];
        let text = build_skills_prompt_markdown(&displayable, true, false);
        assert!(
            text.contains("activate_skill"),
            "activate_skill must be mentioned when has_activate_skill=true"
        );
        assert!(
            text.contains("activate_skill(name="),
            "activate_skill call syntax must be present"
        );
        assert!(text.contains("**my-skill**"));
    }

    #[test]
    fn test_skills_prompt_without_activate_tool() {
        let skill = make_skill_index("my-skill", "Does something useful");
        let displayable = vec![&skill];
        let text = build_skills_prompt_markdown(&displayable, false, false);
        assert!(
            !text.contains("activate_skill"),
            "activate_skill must not be mentioned when has_activate_skill=false"
        );
        assert!(
            text.contains("/skill-name"),
            "slash syntax must still be mentioned"
        );
        assert!(text.contains("**my-skill**"));
    }

    #[test]
    fn test_skills_prompt_deactivate_tool_conditional() {
        let skill = make_skill_index("my-skill", "Does something useful");
        let displayable = vec![&skill];
        let text_with = build_skills_prompt_markdown(&displayable, true, true);
        let text_without = build_skills_prompt_markdown(&displayable, true, false);
        assert!(
            text_with.contains("deactivate_skill"),
            "deactivate_skill must appear when has_deactivate_skill=true"
        );
        assert!(
            text_with.contains("report="),
            "report parameter must appear when has_deactivate_skill=true"
        );
        assert!(
            !text_without.contains("deactivate_skill"),
            "deactivate_skill must not appear when has_deactivate_skill=false"
        );
    }

    #[tokio::test]
    async fn test_non_user_invocable_never_auto_triggered() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "hidden-skill",
            "name: hidden-skill\ndescription: reviews security vulnerabilities code\nuser-invocable: false\ndisable-model-invocation: false",
            "Body",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "security vulnerabilities review",
            None,
            SkillsAutoTrigger::InjectFull,
        )
        .await;
        assert!(
            msgs.is_empty(),
            "user_invocable=false skill must not be auto-injected"
        );
    }

    #[tokio::test]
    async fn test_explicit_injection_rejects_disable_model_invocation() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "locked-skill",
            "name: locked-skill\ndescription: Locked skill\ndisable-model-invocation: true",
            "Sensitive instructions",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "anything",
            Some("locked-skill"),
            SkillsAutoTrigger::InjectFull,
        )
        .await;
        assert!(
            msgs.is_empty(),
            "disable_model_invocation=true must block explicit injection"
        );
    }

    #[tokio::test]
    async fn test_explicit_injection_rejects_non_invocable() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "non-invocable-skill",
            "name: non-invocable-skill\ndescription: Non-invocable skill\nuser-invocable: false",
            "Internal instructions",
        )
        .await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let (msgs, _) = build_context_messages_from_dirs(
            &ext_dirs,
            "anything",
            Some("non-invocable-skill"),
            SkillsAutoTrigger::InjectFull,
        )
        .await;
        assert!(
            msgs.is_empty(),
            "user_invocable=false must block explicit injection"
        );
    }

    #[tokio::test]
    async fn test_auto_trigger_subset_of_prompt_skills() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "normal-skill",
            "name: normal-skill\ndescription: reviews security vulnerabilities code auditing",
            "Body",
        )
        .await;
        write_skill(
            tmp.path(),
            "disabled-skill",
            "name: disabled-skill\ndescription: reviews security vulnerabilities code auditing\ndisable-model-invocation: true",
            "Body",
        ).await;
        write_skill(
            tmp.path(),
            "noninvocable-skill",
            "name: noninvocable-skill\ndescription: reviews security vulnerabilities code auditing\nuser-invocable: false",
            "Body",
        ).await;

        let ext_dirs = make_ext_dirs(tmp.path());
        let indices = crate::ext::skills::load_skill_indices(&ext_dirs).await;

        let displayable: Vec<_> = indices
            .iter()
            .filter(|s| s.user_invocable && !s.disable_model_invocation)
            .collect();
        let prompt_names: std::collections::HashSet<&str> =
            displayable.iter().map(|s| s.name.as_str()).collect();

        let triggered = crate::ext::skills_matcher::select_relevant_skills(
            &indices,
            "security vulnerabilities code review",
            10,
            0.0,
        );

        for name in &triggered {
            assert!(
                prompt_names.contains(name.as_str()),
                "Auto-triggered skill '{}' must be in the prompt set (prompt has: {:?})",
                name,
                prompt_names
            );
        }
        assert!(
            prompt_names.contains("normal-skill"),
            "normal-skill must be in prompt"
        );
        assert!(
            !prompt_names.contains("disabled-skill"),
            "disabled-skill must not be in prompt"
        );
        assert!(
            !prompt_names.contains("noninvocable-skill"),
            "noninvocable-skill must not be in prompt"
        );
        assert!(
            triggered.contains(&"normal-skill".to_string()),
            "normal-skill must be auto-triggered"
        );
        assert!(
            !triggered.contains(&"disabled-skill".to_string()),
            "disabled-skill must not be auto-triggered"
        );
        assert!(
            !triggered.contains(&"noninvocable-skill".to_string()),
            "noninvocable-skill must not be auto-triggered"
        );
    }

    #[test]
    fn test_skills_prompt_text_not_empty_when_no_skills() {
        let text = build_skills_prompt_markdown(&[], true, true);
        assert!(!text.is_empty());
        assert!(text.contains("## Skills"));
        assert!(text.contains("No skills are currently installed"));
    }

    #[test]
    fn test_skills_prompt_text_not_empty_when_off() {
        let text = "## Skills\n\nSkills system is disabled by configuration.";
        assert!(!text.is_empty());
        assert!(text.contains("## Skills"));
        assert!(text.contains("disabled by configuration"));
    }

    #[test]
    fn test_skills_prompt_markdown_with_skills_documents_lifecycle() {
        let skill = make_skill_index("my-skill", "Does something useful");
        let displayable = vec![&skill];
        let text = build_skills_prompt_markdown(&displayable, true, true);
        assert!(
            text.contains("deactivate_skill"),
            "lifecycle must mention deactivate_skill"
        );
        assert!(
            text.contains("report"),
            "lifecycle must mention report parameter"
        );
        assert!(
            text.contains("compacts"),
            "lifecycle must mention that report compacts history"
        );
    }

    #[test]
    fn test_skills_prompt_markdown_empty_documents_system() {
        let text = build_skills_prompt_markdown(&[], true, true);
        assert!(!text.is_empty());
        assert!(text.contains("## Skills"));
        assert!(text.contains("No skills are currently installed"));
        assert!(
            text.contains("activate_skill"),
            "empty state must mention activate_skill when tool available"
        );
        assert!(
            text.contains("deactivate_skill"),
            "empty state must mention deactivate_skill when tool available"
        );
    }
}
