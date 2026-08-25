use super::ToolCellType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolFamily {
    Shell,
    Process,
    Diff,
    Server,
    Mcp,
    WebSearch,
    WebFetch,
    CodeSearch,
    FileSearch,
    TreeSearch,
    DocumentationSearch,
    KnowledgeSearch,
    RequestInput,
    Unknown,
}

const EXACT_TOOL_FAMILIES: &[(&str, ToolFamily)] = &[
    ("shell", ToolFamily::Shell),
    ("patch", ToolFamily::Diff),
    ("apply_patch", ToolFamily::Diff),
    ("text_edit", ToolFamily::Diff),
    ("create_textdoc", ToolFamily::Diff),
    ("update_textdoc", ToolFamily::Diff),
    ("replace_textdoc", ToolFamily::Diff),
    ("update_textdoc_regex", ToolFamily::Diff),
    ("update_textdoc_by_lines", ToolFamily::Diff),
    ("update_textdoc_anchored", ToolFamily::Diff),
    ("undo_textdoc", ToolFamily::Diff),
    ("rm", ToolFamily::Diff),
    ("mv", ToolFamily::Diff),
    ("mcp_call", ToolFamily::Mcp),
    ("web_search", ToolFamily::WebSearch),
    ("web", ToolFamily::WebFetch),
    ("search", ToolFamily::CodeSearch),
    ("search_pattern", ToolFamily::CodeSearch),
    ("search_semantic", ToolFamily::CodeSearch),
    ("search_symbol_definition", ToolFamily::CodeSearch),
    ("cat", ToolFamily::FileSearch),
    ("glob", ToolFamily::FileSearch),
    ("tree", ToolFamily::TreeSearch),
    ("doc_get", ToolFamily::DocumentationSearch),
    ("doc_list", ToolFamily::DocumentationSearch),
    ("knowledge", ToolFamily::KnowledgeSearch),
    ("search_trajectories", ToolFamily::KnowledgeSearch),
    ("task_mem_search", ToolFamily::KnowledgeSearch),
    ("vecdb_search", ToolFamily::KnowledgeSearch),
    ("ask_questions", ToolFamily::RequestInput),
    ("request_user_input", ToolFamily::RequestInput),
    ("request-user-input", ToolFamily::RequestInput),
    ("agent_ask_planner", ToolFamily::RequestInput),
    ("web_search_call", ToolFamily::Server),
    ("file_search_call", ToolFamily::Server),
    ("code_interpreter_call", ToolFamily::Server),
    ("local_shell_call", ToolFamily::Server),
    ("image_generation_call", ToolFamily::Server),
    ("computer_use_call", ToolFamily::Server),
    ("web_fetch", ToolFamily::Server),
    ("code_execution", ToolFamily::Server),
];

const PREFIX_TOOL_FAMILIES: &[(&str, ToolFamily)] = &[
    ("process_", ToolFamily::Process),
    ("srvtoolu_", ToolFamily::Server),
];

pub fn tool_family(name: &str) -> ToolFamily {
    EXACT_TOOL_FAMILIES
        .iter()
        .find_map(|(tool_name, family)| (*tool_name == name).then_some(*family))
        .or_else(|| {
            PREFIX_TOOL_FAMILIES
                .iter()
                .find_map(|(prefix, family)| name.starts_with(prefix).then_some(*family))
        })
        .unwrap_or(ToolFamily::Unknown)
}

impl ToolFamily {
    pub fn cell_type(self) -> ToolCellType {
        match self {
            Self::Shell | Self::Process => ToolCellType::Exec,
            Self::Diff => ToolCellType::Diff,
            Self::Server | Self::Mcp => ToolCellType::Server,
            Self::WebSearch
            | Self::WebFetch
            | Self::CodeSearch
            | Self::FileSearch
            | Self::TreeSearch
            | Self::DocumentationSearch
            | Self::KnowledgeSearch => ToolCellType::Search,
            Self::RequestInput => ToolCellType::RequestInput,
            Self::Unknown => ToolCellType::Generic,
        }
    }

    pub fn search_header(self, active: bool) -> &'static str {
        match (self, active) {
            (Self::WebSearch, true) => "Searching the web",
            (Self::WebSearch, false) => "Searched the web",
            (Self::WebFetch, true) => "Fetching the web",
            (Self::WebFetch, false) => "Fetched the web",
            (Self::CodeSearch, true) => "Searching code",
            (Self::CodeSearch, false) => "Searched code",
            (Self::FileSearch, true) => "Searching files",
            (Self::FileSearch, false) => "Searched files",
            (Self::TreeSearch, true) => "Inspecting the file tree",
            (Self::TreeSearch, false) => "Inspected the file tree",
            (Self::DocumentationSearch, true) => "Searching documentation",
            (Self::DocumentationSearch, false) => "Searched documentation",
            (Self::KnowledgeSearch, true) => "Searching knowledge",
            (Self::KnowledgeSearch, false) => "Searched knowledge",
            (_, true) => "Searching",
            (_, false) => "Searched",
        }
    }

    pub fn is_shell(self) -> bool {
        self == Self::Shell
    }

    pub fn is_mcp(self) -> bool {
        self == Self::Mcp
    }
}
