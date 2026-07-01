pub const CATEGORY_CODE: &str = "code";
pub const CATEGORY_CONFIG: &str = "config";
pub const CATEGORY_DOC: &str = "doc";
pub const CATEGORY_DATA: &str = "data";
pub const CATEGORY_PIPELINE: &str = "pipeline";

const DOC_SUFFIXES: &[&str] = &[".md", ".mdx", ".rst", ".txt", ".adoc"];
const CONFIG_LANGUAGES: &[&str] = &["yaml", "toml", "json", "ini", "properties", "hcl"];
const DATA_DIR_TOKENS: &[&str] = &[
    "migrations",
    "versions",
    "models",
    "schema",
    "schemas",
    "entities",
];
const DATA_SUFFIXES: &[&str] = &[".sql", ".prisma", ".graphql", ".proto"];
const PIPELINE_PATH_HINTS: &[&str] = &[
    ".github/workflows/",
    ".gitlab-ci",
    "jenkinsfile",
    "azure-pipelines",
    ".circleci/",
    "/pipelines/",
    "/etl/",
];

pub fn file_category(path: &str, language: &str, is_config: bool) -> &'static str {
    let suffix = suffix(path);

    if DOC_SUFFIXES.contains(&suffix.as_str()) {
        return CATEGORY_DOC;
    }

    let lower_path = path.replace('\\', "/").to_lowercase();
    if PIPELINE_PATH_HINTS
        .iter()
        .any(|hint| lower_path.contains(hint))
    {
        return CATEGORY_PIPELINE;
    }

    let parent_segments = parent_segments(path);
    if DATA_SUFFIXES.contains(&suffix.as_str())
        || parent_segments
            .iter()
            .any(|segment| DATA_DIR_TOKENS.contains(&segment.as_str()))
    {
        return CATEGORY_DATA;
    }

    let lower_language = language.to_lowercase();
    if is_config || CONFIG_LANGUAGES.contains(&lower_language.as_str()) {
        return CATEGORY_CONFIG;
    }

    CATEGORY_CODE
}

pub fn well_known_role(path: &str) -> Option<&'static str> {
    let name = basename(path).to_lowercase();

    if let Some(role) = role_by_name(&name) {
        return Some(role);
    }

    if name.ends_with(".lock") {
        return Some("Resolved dependency lockfile pinning exact versions.");
    }

    let parent_segments = parent_segments(path);
    if parent_segments
        .windows(2)
        .any(|segments| segments == [".github", "workflows"])
    {
        return Some("GitHub Actions CI/CD workflow definition.");
    }

    None
}

fn suffix(path: &str) -> String {
    let name = basename(path);
    match name.rfind('.') {
        Some(index) if index < name.len() - 1 => name[index..].to_lowercase(),
        _ => String::new(),
    }
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or("")
}

fn parent_segments(path: &str) -> Vec<String> {
    let mut parts: Vec<&str> = path.split(['/', '\\']).collect();
    parts.pop();
    parts.into_iter().map(|part| part.to_lowercase()).collect()
}

fn role_by_name(name: &str) -> Option<&'static str> {
    match name {
        "pyproject.toml" => Some("Python project metadata, dependencies, and build configuration."),
        "setup.py" => Some("Python package build script (setuptools)."),
        "setup.cfg" => Some("Python packaging and tool configuration."),
        "requirements.txt" => Some("Pinned Python dependency list."),
        "tox.ini" => Some("Tox test-automation and environment matrix configuration."),
        "ruff.toml" => Some("Ruff linter and formatter configuration."),
        "mypy.ini" => Some("MyPy static type-checking configuration."),
        "pytest.ini" => Some("Pytest configuration."),
        "conftest.py" => Some("Shared pytest fixtures and test configuration."),
        "alembic.ini" => Some("Alembic database-migration configuration."),
        "package.json" => Some("Node package manifest: dependencies, scripts, and metadata."),
        "package-lock.json" => Some("Resolved npm dependency lockfile."),
        "tsconfig.json" => Some("TypeScript compiler configuration."),
        "vite.config.ts" => Some("Vite build and dev-server configuration."),
        "vite.config.js" => Some("Vite build and dev-server configuration."),
        "next.config.js" => Some("Next.js framework configuration."),
        "next.config.mjs" => Some("Next.js framework configuration."),
        "tailwind.config.ts" => Some("Tailwind CSS design-token and theme configuration."),
        "tailwind.config.js" => Some("Tailwind CSS design-token and theme configuration."),
        "eslint.config.js" => Some("ESLint linting rules."),
        ".eslintrc.json" => Some("ESLint linting rules."),
        ".prettierrc" => Some("Prettier formatting configuration."),
        "dockerfile" => Some("Container image build definition."),
        "docker-compose.yml" => {
            Some("Multi-container service orchestration for local development.")
        }
        "docker-compose.yaml" => {
            Some("Multi-container service orchestration for local development.")
        }
        ".dockerignore" => Some("Files excluded from the Docker build context."),
        "makefile" => Some("Build, test, and task automation targets."),
        ".gitignore" => Some("Paths excluded from version control."),
        ".gitattributes" => Some("Per-path Git behaviour (line endings, diff, linguist)."),
        ".editorconfig" => Some("Editor formatting conventions shared across the team."),
        ".pre-commit-config.yaml" => Some("Pre-commit hook definitions run before each commit."),
        "readme.md" => Some("Project overview and entry point for new readers."),
        "contributing.md" => Some("How to contribute: workflow, standards, and review process."),
        "security.md" => Some("Security policy and vulnerability-reporting process."),
        "code_of_conduct.md" => Some("Community code of conduct."),
        "license" => Some("Project license terms."),
        "license.md" => Some("Project license terms."),
        "changelog.md" => Some("Release history and notable changes."),
        "codeowners" => Some("Path-to-reviewer ownership mapping."),
        "pull_request_template.md" => {
            Some("Template prompting authors for PR description and checklist.")
        }
        "bug_report.md" => Some("Issue template for reporting bugs."),
        "feature_request.md" => Some("Issue template for proposing features."),
        "funding.yml" => Some("Sponsorship and funding links for the repository."),
        "marketplace.json" => Some("Plugin marketplace listing and metadata."),
        "plugin.json" => Some("Plugin manifest: identity, entry points, and capabilities."),
        "hooks.json" => Some("Plugin lifecycle hook definitions."),
        "claude.md" => Some("Repository instructions and context for AI coding agents."),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_document_by_suffix() {
        assert_eq!(file_category("README.md", "", false), CATEGORY_DOC);
    }

    #[test]
    fn categorizes_pipeline_by_path_hint() {
        assert_eq!(
            file_category(".github/workflows/ci.yml", "", false),
            CATEGORY_PIPELINE
        );
    }

    #[test]
    fn categorizes_data_by_parent_dir_token() {
        assert_eq!(
            file_category("db/migrations/0001.py", "", false),
            CATEGORY_DATA
        );
    }

    #[test]
    fn categorizes_config_by_language() {
        assert_eq!(file_category("config.yaml", "yaml", false), CATEGORY_CONFIG);
    }

    #[test]
    fn categorizes_code_by_default() {
        assert_eq!(file_category("src/main.rs", "", false), CATEGORY_CODE);
    }

    #[test]
    fn finds_role_by_lowercased_basename() {
        assert_eq!(
            well_known_role("src/Dockerfile"),
            Some("Container image build definition.")
        );
    }

    #[test]
    fn finds_lockfile_role_by_suffix() {
        assert_eq!(
            well_known_role("x/y.lock"),
            Some("Resolved dependency lockfile pinning exact versions.")
        );
    }

    #[test]
    fn finds_github_workflow_role_by_parent_segments() {
        assert_eq!(
            well_known_role(".github/workflows/ci.yml"),
            Some("GitHub Actions CI/CD workflow definition.")
        );
    }

    #[test]
    fn returns_none_for_unknown_path() {
        assert_eq!(well_known_role("src/main.rs"), None);
    }
}
