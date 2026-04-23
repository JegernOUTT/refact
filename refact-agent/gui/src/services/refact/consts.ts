export const CAPS_URL = `/v1/caps`;
export const STATISTIC_URL = `/v1/get-dashboard-plots`;
export const AT_COMMAND_COMPLETION = "/v1/at-command-completion";
export const AT_COMMAND_PREVIEW = "/v1/at-command-preview";
export const CUSTOM_PROMPTS_URL = "/v1/customization";
export const TOOLS = "/v1/tools";
export const TOOLS_CHECK_CONFIRMATION =
  "/v1/tools-check-if-confirmation-needed";
export const EDIT_TOOL_DRY_RUN_URL = "/v1/file_edit_tool_dry_run";
export const CONFIG_PATH_URL = "/v1/config-path";
export const FULL_PATH_URL = "/v1/fullpath";
// TODO: add a service for the docs.
export const DOCUMENTATION_LIST = `/v1/docs-list`;
export const DOCUMENTATION_ADD = `/v1/docs-add`;
export const DOCUMENTATION_REMOVE = `/v1/docs-remove`;
export const PING_URL = `/v1/ping`;
export const PATCH_URL = `/v1/patch-single-file-from-ticket`;
export const APPLY_ALL_URL = "/v1/patch-apply-all";
export const CHAT_LINKS_URL = "/v1/links";
export const CHAT_COMMIT_LINK_URL = "/v1/git-commit";
// Integrations
export const INTEGRATIONS_URL = "/v1/integrations";
export const INTEGRATION_GET_URL = "/v1/integration-get";
export const INTEGRATION_MCP_LOGS_PATH = "/v1/integrations-mcp-logs";
export const INTEGRATION_SAVE_URL = "/v1/integration-save";
export const INTEGRATION_DELETE_URL = "/v1/integration-delete";
// Agent rollback endpoints
export const PREVIEW_CHECKPOINTS = "/v1/checkpoints-preview";
export const RESTORE_CHECKPOINTS = "/v1/checkpoints-restore";

export const TELEMETRY_CHAT_PATH = "/v1/telemetry-chat";
export const TELEMETRY_NET_PATH = "/v1/telemetry-network";

export const COMPRESS_MESSAGES_URL = "/v1/trajectory-compress";

export const SET_ACTIVE_GROUP_ID = "/v1/set-active-group-id";

export const TRAJECTORY_TRANSFORM_PREVIEW_URL =
  "/v1/chats/{chat_id}/trajectory/transform/preview";
export const TRAJECTORY_TRANSFORM_APPLY_URL =
  "/v1/chats/{chat_id}/trajectory/transform/apply";
export const TRAJECTORY_HANDOFF_PREVIEW_URL =
  "/v1/chats/{chat_id}/trajectory/handoff/preview";
export const TRAJECTORY_HANDOFF_APPLY_URL =
  "/v1/chats/{chat_id}/trajectory/handoff/apply";
export const TRAJECTORY_MODE_TRANSITION_APPLY_URL =
  "/v1/chats/{chat_id}/trajectory/mode-transition/apply";

// Providers & Models (new provider system)
export const PROVIDERS_URL = "/v1/providers";
export const PROVIDER_DEFAULTS_URL = "/v1/defaults";
// Legacy - kept for backward compatibility
export const CONFIGURED_PROVIDERS_URL = "/v1/providers";
export const PROVIDER_TEMPLATES_URL = "/v1/provider-templates";
export const PROVIDER_URL = "/v1/provider";

export const MODELS_URL = "/v1/models";
export const MODEL_URL = "/v1/model";
export const MODEL_DEFAULTS_URL = "/v1/model-defaults";
export const COMPLETION_MODEL_FAMILIES_URL = "/v1/completion-model-families";

// Browser endpoints
export const BROWSER_START = "/v1/browser/start";
export const BROWSER_STOP = "/v1/browser/stop";
export const BROWSER_SCREENSHOT = "/v1/browser/screenshot";
export const BROWSER_CONTEXT = "/v1/browser/context";
export const BROWSER_CURL = "/v1/browser/curl";
export const BROWSER_ELEMENT_PICK = "/v1/browser/element-pick";
export const BROWSER_ELEMENT_PICK_RESULT = "/v1/browser/element-pick/result";
export const BROWSER_RECORD_ANIMATION = "/v1/browser/record-animation";
export const BROWSER_HANDOFF = "/v1/browser/handoff";
export const BROWSER_STATUS = "/v1/browser/status";
export const BROWSER_CONTEXT_ESTIMATE = "/v1/browser/context-estimate";
export const BROWSER_ANNOTATE_START = "/v1/browser/annotate/start";
export const BROWSER_ANNOTATE_RESULT = "/v1/browser/annotate/result";
export const BROWSER_ANNOTATE_CLEAR = "/v1/browser/annotate/clear";
export const BROWSER_ACTION = "/v1/browser/action";

export const SKILLS_STATUS_URL = "/v1/chats/{chat_id}/skills-status";
