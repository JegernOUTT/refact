use std::sync::Arc;

use crate::global_context::GlobalContext;
use crate::subchat::ExplicitSubchatSpec;
use crate::tools::review_agents::agentic::{
    build_agent_task_prompt, run_agentic_instance, AgenticInstance,
};
use crate::tools::review_agents::config::BrowserSection;
use crate::tools::review_agents::{AgentCtx, AgentOutcome};
use crate::tools::review_scope::ReviewScope;

pub const AGENT_ID: &str = "a4_browser";

pub struct BrowserAgentInput {
    pub slot_label: String,
    pub spec: ExplicitSubchatSpec,
    pub system_prompt: String,
    pub tools: Vec<String>,
    pub max_steps: usize,
    pub section: BrowserSection,
}

async fn chrome_available(gcx: Arc<GlobalContext>) -> bool {
    crate::tools::tools_list::get_available_tools(gcx)
        .await
        .into_iter()
        .any(|tool| tool.tool_description().name == "chrome")
}

pub(crate) async fn run_browser_agent(
    gcx: Arc<GlobalContext>,
    ctx: AgentCtx,
    input: BrowserAgentInput,
    scope: Arc<ReviewScope>,
) -> AgentOutcome {
    if !chrome_available(gcx.clone()).await {
        return AgentOutcome::skipped(
            &format!("{AGENT_ID}@{}", input.slot_label),
            "chrome_unavailable",
        );
    }

    let mut extra = String::new();
    extra.push_str("# Browser review environment\n");
    match input.section.app_url.as_deref() {
        Some(url) => extra.push_str(&format!("Configured app URL: {url}\n")),
        None => extra.push_str("No app URL is configured.\n"),
    }
    match input.section.dev_server_command.as_deref() {
        Some(command) => extra.push_str(&format!("Configured dev-server command: `{command}`\n")),
        None => extra.push_str(
            "No dev-server command is configured; detect one from package.json scripts (dev/start/serve) or equivalent project files.\n",
        ),
    }
    if input.section.allow_dev_server_boot {
        extra.push_str(
            "You MAY boot the dev server yourself: start it with process_start in service mode, wait for the port or a readiness line, then drive the UI. Kill the service with process_kill before finishing.\n",
        );
    } else {
        extra.push_str(
            "Booting a dev server is NOT allowed. If no reachable app URL exists, finish with an empty candidates array and explain why in the summary.\n",
        );
    }
    extra.push_str(
        "\nAttach browser evidence to every candidate: use evidence kind `console_log` for console errors and `screenshot` for visual defects, with textual content describing what was captured.\n",
    );

    let task_prompt = build_agent_task_prompt(&scope, Some(&extra));
    let instance = AgenticInstance {
        agent_id: AGENT_ID.to_string(),
        slot_label: input.slot_label,
        spec: input.spec,
        system_prompt: input.system_prompt,
        task_prompt,
        tools: input.tools,
        max_steps: input.max_steps,
        title: "Review: Browser Agent".to_string(),
        verify: false,
    };
    run_agentic_instance(gcx, ctx, instance, scope, None).await
}
