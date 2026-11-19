use refact_python_catalog::{build_catalog, CatalogOptions, PythonCatalog};
use refact_tool_api::ToolDesc;

pub const INTRO: &str = "Run Python 3 (Monty subset), including top-level await, to compose tools. The last expression is returned. Nested tools return ToolResult envelopes (ok, data, text, files, diffs, images, error, truncated). PermissionError means a call was denied; TypeError means invalid arguments. Variables persist per chat; restart clears variables, not files. A long run continues in the background; its result arrives in this chat by itself and starts your next turn, so do not poll for it.\n\n";
pub fn catalog(tools: &[ToolDesc], mcp_lazy: bool, max_chars: usize) -> PythonCatalog {
    catalog_with_llm(tools, mcp_lazy, max_chars, false)
}
pub fn catalog_with_llm(
    tools: &[ToolDesc],
    mcp_lazy: bool,
    max_chars: usize,
    llm: bool,
) -> PythonCatalog {
    catalog_with_options(
        tools,
        mcp_lazy,
        BindOptions {
            max_chars,
            llm,
            engine: false,
        },
    )
}
/// `options.engine` binds and describes `engine()` with the registered engine functions.
pub fn catalog_with_options(
    tools: &[ToolDesc],
    mcp_lazy: bool,
    options: BindOptions,
) -> PythonCatalog {
    build_catalog(
        tools,
        &CatalogOptions {
            mcp_lazy,
            max_chars: options.max_chars,
            llm: options.llm,
            engine: if options.engine {
                super::engine_fns::docs()
            } else {
                Vec::new()
            },
        },
    )
}
/// Catalog options that depend on project settings.
#[derive(Clone, Copy, Debug)]
pub struct BindOptions {
    pub max_chars: usize,
    pub llm: bool,
    /// `python_engine_read_enabled`: bind and describe `engine()`.
    pub engine: bool,
}
impl BindOptions {
    pub fn from_settings(settings: &refact_work_core::CoordinationSettings) -> Self {
        Self {
            max_chars: settings.python_max_catalog_chars,
            llm: settings.python_max_llm_calls > 0,
            engine: settings.python_engine_read_enabled,
        }
    }
    pub async fn for_project(gcx: crate::global_context::SharedGlobalContext) -> Self {
        let settings = match crate::files_correction::get_active_project_path(gcx).await {
            Some(root) => crate::runtime_settings::coordination_for_project(&root)
                .await
                .ok(),
            None => None,
        }
        .unwrap_or_else(refact_work_core::coordination_settings);
        Self::from_settings(&settings)
    }
    /// A run binds the helpers the tool description advertises (built by `for_project`),
    /// even when its scope root has other settings; budgets and switches stay the run's own.
    pub async fn for_run(
        gcx: crate::global_context::SharedGlobalContext,
        settings: &refact_work_core::CoordinationSettings,
    ) -> Self {
        let described = Self::for_project(gcx).await;
        Self {
            llm: described.llm,
            engine: described.engine,
            ..Self::from_settings(settings)
        }
    }
}
pub fn bind(
    tools: &mut Vec<Box<dyn crate::tools::tools_description::Tool + Send>>,
    lazy: bool,
    options: BindOptions,
) {
    let descriptions: Vec<_> = tools.iter().map(|t| t.tool_description()).collect();
    let description = format!(
        "{INTRO}{}",
        catalog_with_options(&descriptions, lazy, options).description
    );
    for tool in tools.iter_mut() {
        if tool.tool_description().name == "python" {
            *tool = Box::new(crate::tools::tool_python::ToolPython {
                description: description.clone(),
            });
        }
    }
}
