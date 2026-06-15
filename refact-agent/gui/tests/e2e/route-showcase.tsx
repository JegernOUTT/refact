import React from "react";
import { createRoot } from "react-dom/client";
import { Provider, useSelector } from "react-redux";
import { Flex } from "@radix-ui/themes";

import { setUpStore, type RootState } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { Dashboard } from "../../src/features/Dashboard";
import { Chat } from "../../src/features/Chat";
import { TrajectoryButton } from "../../src/components/Trajectory";
import { ModeSelect } from "../../src/components/ChatForm/ModeSelect";
import {
  ModelSelector,
  type ModelOption,
} from "../../src/components/ui/ModelSelector";
import { BuddyHome } from "../../src/features/Buddy";
import { setBuddySnapshot } from "../../src/features/Buddy/buddySlice";
import {
  SETTINGS_SECTIONS,
  SettingsHub,
  settingsSectionToPage,
  type SettingsSectionId,
} from "../../src/features/Settings";
import {
  MarketplaceHub,
  isMarketplacePage,
  marketplaceTabToPage,
  type MarketplaceTabId,
} from "../../src/features/MarketplaceHub";
import { setBackendStatus } from "../../src/features/Connection";
import { sidebarSectionSnapshotReceived } from "../../src/features/Sidebar/sidebarSlice";
import { tasksApi, type TaskMeta } from "../../src/services/refact/tasks";
import type { ChatHistoryItem } from "../../src/features/History/historySlice";
import type { ChatThreadRuntime } from "../../src/features/Chat/Thread";
import type { ChatMessages } from "../../src/services/refact";
import type {
  BuddyActivityEntry,
  BuddyConversationEntry,
  BuddyOpportunity,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySettings,
  BuddySnapshot,
} from "../../src/features/Buddy/types";
import type {
  CapsResponse,
  ConfiguredProvidersResponse,
  IntegrationWithIconResponse,
  ProviderDefaults,
} from "../../src/services/refact";
import type { ConfigItem } from "../../src/services/refact/customization";
import type { ExtRegistryResponse } from "../../src/services/refact/extensions";
import "../../src/lib/render/web.css";

const now = "2026-06-07T10:00:00Z";
const chatId = "showcase-chat";

const buddySettings: BuddySettings = {
  enabled: true,
  auto_diagnostics: true,
  auto_issue_creation: false,
  personality_prompt: null,
  autonomous_chats_enabled: true,
  proactive_enabled: true,
  message_observation_enabled: true,
  chat_reactions_enabled: true,
  housekeeping_enabled: true,
  humor_enabled: true,
  humor_level: "light",
  autonomy_level: "suggest",
  quiet_mode: false,
  daily_digest_hour: 18,
  observers: {
    task_health: true,
    trajectory_clutter: true,
    chat_pattern: true,
    customization_drift: true,
    memory_garden: true,
    mcp_auth: true,
    git_pressure: true,
    diagnostic_cluster: true,
    provider_health: true,
  },
};

const buddyPulse: BuddyPulse = {
  generated_at: now,
  tasks: { total: 48, stuck: 1, abandoned: 9, by_status: { planning: 12 } },
  trajectories: { total: 124, untitled: 6, oldest_age_days: 18 },
  memory: { total: 28156, orphan: 3, stale_conflicts: 1 },
  providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 1 },
  mcp: { total: 7, failing: 1, auth_expiring: 1 },
  customization: { modes: 8, skills: 14, commands: 9, subagents: 6, hooks: 3 },
  diagnostics: { last_hour: 4, top_error_types: ["frontend", "llm_error"] },
  git: { uncommitted_files: 5, diff_lines_4h: 180, branches: 4 },
  worktrees: {
    total_registered: 5,
    total_discovered: 2,
    total: 7,
    clean: 3,
    dirty: 2,
    unknown: 0,
    stale: 1,
    conflicted: 0,
    shared: 1,
    abandoned_clean: 2,
    changed_files: 9,
    additions: 420,
    deletions: 73,
    missing_registry_paths: 1,
    unregistered_cache_dirs: 1,
    merged_branches: 2,
  },
  humor: "Tiny dashboard goblin found one suspicious scrollbar crumb.",
};

const buddyOpportunity: BuddyOpportunity = {
  id: "showcase-opportunity",
  kind: "diagnostic_investigation",
  summary:
    "Investigate a long-running diagnostic cluster before it becomes gremlin soup",
  priority: "high",
  confidence: 0.92,
  fact_keys: ["diagnostic:frontend:overflow"],
  cooldown_key: "showcase-opportunity",
  cooldown_secs: 1800,
  status: "new",
  proposed_actions: [
    { kind: "open_page", page: { type: "providers" } },
    { kind: "dismiss" },
  ],
  humor_allowed: true,
  humor: "Scrollbar? I hardly knowbar.",
  related: {
    chat_ids: ["showcase-chat"],
    task_ids: ["task-active"],
    memory_ids: [],
    config_paths: ["/workspace/refact/refact-agent/gui/src/features/Buddy"],
  },
  created_at: now,
  expires_at: "2099-12-31T00:00:00Z",
};

const buddyActivities: BuddyActivityEntry[] = [
  {
    icon: "🧭",
    title: "Buddy reviewed responsive surfaces",
    description:
      "Checked hero, grids, activity, settings, and workshop panels.",
    timestamp: now,
    activity_type: "buddy_layout_review",
    chat_id: "showcase-chat",
  },
  {
    icon: "⚙️",
    title: "Refact e2e gate queued",
    description: "No horizontal scroll matrix is ready for narrow widths.",
    timestamp: "2026-06-07T09:40:00Z",
    activity_type: "refact_e2e_gate",
    chat_id: null,
  },
];

const buddyRuntimeEvent: BuddyRuntimeEvent = {
  id: "showcase-runtime-error",
  signal_type: "frontend_error_burst",
  title: "Possible layout overflow",
  description: "A narrow viewport almost clipped a Buddy utility panel.",
  source: "route-showcase",
  status: "failed",
  failure_category: "frontend",
  failure_summary: "responsive layout guard",
  priority: "high",
  created_at: now,
  chat_id: "showcase-chat",
};

const buddyConversations: BuddyConversationEntry[] = [
  {
    id: "showcase-buddy-chat",
    kind: "chat",
    title: "Buddy route responsive smoke with a very long title",
    created_at: now,
    updated_at: now,
    status: "completed",
    message_count: 7,
    icon: "💬",
    badge: "Review",
  },
  {
    id: "showcase-buddy-workflow",
    kind: "workflow",
    title: "Autonomous layout watcher",
    created_at: "2026-06-07T08:00:00Z",
    updated_at: "2026-06-07T08:30:00Z",
    status: "completed",
    message_count: 3,
    icon: "🛠️",
    badge: "Workflow",
    workflow_id: "buddy_layout_watcher",
  },
];

const buddySnapshot: BuddySnapshot = {
  state: {
    identity: {
      name: "Pixel",
      created_at: "2026-06-01T00:00:00Z",
      palette_index: 0,
    },
    progression: {
      stage: 1,
      stage_name: "Sprout",
      level: 3,
      xp: 42,
      xp_next: 80,
    },
    skills: { unlocked: ["review", "nudge"], locked: ["teleport"] },
    workflow_summaries: [],
    semantic: {
      mood: "curious",
      focus: "responsive polish",
      headline: "Guarding narrow layouts with tiny claws",
      last_active: now,
    },
    recent_activities: buddyActivities,
    suggestion_state: [
      {
        id: "showcase-suggestion",
        suggestion_type: "layout_check",
        title: "Run a narrow Buddy smoke pass",
        description: "Open settings and filter activity after the route loads.",
        created_at: now,
        dismissed: false,
        controls: [],
        quest: null,
      },
    ],
    pet: {
      needs: {
        hunger: 70,
        energy: 82,
        hygiene: 88,
        boredom: 18,
        affection: 91,
      },
      condition: {
        sleeping: false,
        hungry: false,
        sleepy: false,
        dirty: false,
        bored: false,
        lonely: false,
      },
      evolution: {
        care_score: 12,
        neglect_score: 1,
        open_seconds: 320,
        last_evolved_at: null,
      },
    },
    personality: {
      archetype_id: "chaos_sprite",
      archetype_label: "Chaos Sprite",
      vibe: "Mildly chaotic, cute, and helpful",
      summary: "A tiny gremlin assistant who celebrates completed checks.",
      prompt: "Be warm, curious, and gently mischievous.",
      traits: {
        playfulness: 82,
        chaos: 48,
        sociability: 74,
        curiosity: 88,
        resilience: 69,
      },
    },
    active_quest: null,
    opportunities: [buddyOpportunity],
  },
  settings: buddySettings,
  enabled: true,
  storage: {
    project_root: "/workspace/refact",
    buddy_dir: "/workspace/refact/.refact/buddy",
    settings_path: "/workspace/refact/.refact/buddy/settings.json",
  },
  recent_diagnostics: [],
  active_speech: null,
  runtime_queue: [buddyRuntimeEvent],
  now_playing: null,
  pulse: buddyPulse,
  opportunities: [buddyOpportunity],
  active_drafts: [],
  chat_reaction_debug: {
    recent_attempts: [],
    counts_by_result: { emitted: 2, skipped: 1 },
    last_skip_reason: null,
    last_emitted_at: now,
  },
};

const showcaseMessages: ChatMessages = [
  {
    role: "user",
    message_id: "showcase-user-1",
    content:
      "Check that expandable tool cards and message footer actions stay responsive.",
  },
  {
    role: "assistant",
    message_id: "showcase-assistant-tools",
    content:
      "I checked two representative tool families so this route can verify expand and collapse interactions.",
    tool_calls: [
      {
        id: "showcase-read-tool",
        index: 0,
        type: "function",
        function: {
          name: "cat",
          arguments: JSON.stringify({
            paths: "src/components/ui/ToolCard.tsx",
          }),
        },
      },
      {
        id: "showcase-exec-tool",
        index: 1,
        type: "function",
        function: {
          name: "process_start",
          arguments: JSON.stringify({
            command: "npm run build",
            description: "Build route showcase",
            mode: "background",
          }),
        },
      },
    ],
    usage: {
      prompt_tokens: 1200,
      completion_tokens: 140,
      total_tokens: 1340,
    },
  },
  {
    role: "tool",
    tool_call_id: "showcase-read-tool",
    content:
      "File src/components/ui/ToolCard.tsx:1-8\n```tsx\nexport function ToolCard() {\n  return <section />;\n}\n```",
    tool_failed: false,
  },
  {
    role: "tool",
    tool_call_id: "showcase-exec-tool",
    content:
      "Process started\nstdout:\nroute showcase ready\nstderr:\n<empty>\n",
    tool_failed: false,
    extra: {
      process_id: "exec_route_showcase",
      status: "running",
      short_description: "Build route showcase",
      command: "npm run build",
      mode: "background",
      cwd: "/workspace/refact-agent/gui",
      started_at_ms: Date.now() - 12_000,
    },
  },
];

const chatRuntime: ChatThreadRuntime = {
  thread: {
    id: chatId,
    title: "Responsive route showcase chat",
    createdAt: now,
    updatedAt: now,
    model: "gpt-4o",
    tool_use: "agent",
    messages: showcaseMessages,
    new_chat_suggested: { wasSuggested: false },
  },
  streaming: false,
  waiting_for_response: false,
  prevent_send: false,
  error: null,
  queued_items: [],
  send_immediately: false,
  attached_images: [],
  attached_text_files: [],
  background_agents: {},
  confirmation: {
    pause: false,
    pause_reasons: [],
    status: { wasInteracted: false, confirmationStatus: true },
  },
  snapshot_received: true,
  task_widget_expanded: false,
  memory_enrichment_user_touched: false,
  manual_preview_items: [],
  manual_preview_ran: false,
};

const historyItem = (
  id: string,
  title: string,
  updatedAt: string,
): ChatHistoryItem => ({
  id,
  title,
  createdAt: updatedAt,
  updatedAt,
  model: "gpt-4o",
  messages: [],
  new_chat_suggested: { wasSuggested: false },
  message_count: 3,
  mode: "agent",
});

const tasks: TaskMeta[] = [
  {
    id: "task-planning",
    name: "Responsive design-system migration",
    status: "planning",
    created_at: now,
    updated_at: now,
    cards_total: 37,
    cards_done: 5,
    cards_failed: 0,
    agents_active: 1,
  },
  {
    id: "task-active",
    name: "Token layer audit",
    status: "active",
    created_at: now,
    updated_at: now,
    cards_total: 8,
    cards_done: 3,
    cards_failed: 1,
    agents_active: 2,
  },
];

const settingsSectionParam = new URLSearchParams(window.location.search).get(
  "settings",
);
const settingsSectionIds = new Set<string>(
  SETTINGS_SECTIONS.map((section) => section.id),
);
const settingsSection: SettingsSectionId = settingsSectionIds.has(
  settingsSectionParam ?? "",
)
  ? (settingsSectionParam as SettingsSectionId)
  : "general";
const marketplaceTabParam = new URLSearchParams(window.location.search).get(
  "marketplace",
);
const marketplaceTabs = new Set<string>([
  "skills",
  "commands",
  "subagents",
  "mcp",
  "extensions",
]);
const marketplaceTab: MarketplaceTabId = marketplaceTabs.has(
  marketplaceTabParam ?? "",
)
  ? (marketplaceTabParam as MarketplaceTabId)
  : "skills";

const providerList: ConfiguredProvidersResponse = {
  providers: [
    {
      name: "openai_codex_personal",
      base_provider: "openai",
      display_name: "OpenAI Codex Personal With A Very Long Display Name",
      enabled: true,
      readonly: false,
      has_credentials: true,
      status: "active",
      model_count: 12,
    },
    {
      name: "local_experimental_provider_with_long_name",
      base_provider: "openai_compatible",
      display_name: "Local Experimental Provider With Long Name",
      enabled: false,
      readonly: false,
      has_credentials: false,
      status: "configured",
      model_count: 3,
    },
  ],
  error_log: [],
};

const chatModel = (id: string) => ({
  n_ctx: 128000,
  name: id,
  tokenizer: "cl100k_base",
  id,
  supports_tools: true,
  supports_multimodality: true,
  supports_clicks: false,
  supports_agent: true,
  reasoning_effort_options: ["low", "medium", "high"],
  supports_thinking_budget: true,
  supports_adaptive_thinking_budget: false,
  default_temperature: 0.2,
  enabled: true,
  type: "chat" as const,
});

const caps: CapsResponse = {
  caps_version: 1,
  chat_default_model: "openai_codex_personal/gpt-5.5",
  chat_model_2: "openai_codex_personal/gpt-5.5-mini",
  task_planner_agent_model: "openai_codex_personal/gpt-5.5-planner",
  chat_thinking_model: "openai_codex_personal/gpt-5.5-thinking",
  chat_light_model:
    "local_experimental_provider_with_long_name/tiny-but-useful-model",
  chat_buddy_model: "openai_codex_personal/gpt-5.5-buddy",
  chat_models: {
    "openai_codex_personal/gpt-5.5": chatModel("openai_codex_personal/gpt-5.5"),
    "openai_codex_personal/gpt-5.5-mini": chatModel(
      "openai_codex_personal/gpt-5.5-mini",
    ),
    "openai_codex_personal/gpt-5.5-planner": chatModel(
      "openai_codex_personal/gpt-5.5-planner",
    ),
    "openai_codex_personal/gpt-5.5-thinking": chatModel(
      "openai_codex_personal/gpt-5.5-thinking",
    ),
    "openai_codex_personal/gpt-5.5-buddy": chatModel(
      "openai_codex_personal/gpt-5.5-buddy",
    ),
    "local_experimental_provider_with_long_name/tiny-but-useful-model":
      chatModel(
        "local_experimental_provider_with_long_name/tiny-but-useful-model",
      ),
  },
  code_chat_default_system_prompt: "",
  completion_models: {},
  completion_default_model: "",
  code_completion_n_ctx: 8192,
  endpoint_chat_passthrough: "",
  endpoint_style: "openai",
  endpoint_template: "",
  running_models: [],
  tokenizer_path_template: "",
  tokenizer_rewrite_path: {},
  metadata: { pricing: {} },
  customization: "",
};

const defaults: ProviderDefaults = {
  chat: { model: "openai_codex_personal/gpt-5.5", max_new_tokens: 4096 },
  chat_model_2: { model: "openai_codex_personal/gpt-5.5-mini" },
  task_planner_agent_model: {
    model: "openai_codex_personal/gpt-5.5-planner",
  },
  chat_light: {
    model: "local_experimental_provider_with_long_name/tiny-but-useful-model",
  },
  chat_thinking: { model: "openai_codex_personal/gpt-5.5-thinking" },
  chat_buddy: { model: "openai_codex_personal/gpt-5.5-buddy" },
};

const overlayRegressionModels: ModelOption[] = [
  {
    value: "openai_codex_personal/gpt-5.5",
    displayName: "OpenAI Codex Personal GPT 5.5",
    group: "openai_codex_personal",
    badges: ["default"],
    contextWindow: "128K ctx",
  },
  {
    value: "openai_codex_personal/gpt-5.5-planner",
    displayName: "Task Planner GPT 5.5",
    group: "openai_codex_personal",
    badges: ["task-agent"],
    contextWindow: "128K ctx",
  },
  {
    value: "local_experimental_provider_with_long_name/tiny-but-useful-model",
    displayName: "Tiny Useful Local Model",
    group: "local_experimental_provider_with_long_name",
    badges: ["light"],
    contextWindow: "32K ctx",
  },
];

const overlayRegressionModelGroups = [
  { id: "openai_codex_personal", label: "OpenAI Codex Personal" },
  {
    id: "local_experimental_provider_with_long_name",
    label: "Local Experimental Provider",
  },
];

const chatModesResponse = {
  modes: [
    {
      id: "agent",
      title: "Agent",
      description: "Autonomous coding mode",
      tools_count: 12,
      thread_defaults: {
        include_project_info: true,
        checkpoints_enabled: true,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
      },
      ui: { order: 1, tags: ["editing", "tools"] },
    },
    {
      id: "ask",
      title: "Ask",
      description: "Quick answers without edits",
      tools_count: 0,
      thread_defaults: {
        include_project_info: true,
        checkpoints_enabled: false,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
      },
      ui: { order: 2, tags: ["chat"] },
    },
  ],
  errors: [],
};

const configItem = (
  kind: ConfigItem["kind"],
  id: string,
  title: string,
): ConfigItem => ({
  id,
  kind,
  title,
  file_path: `/tmp/${id}.yaml`,
  specific: false,
  scope: "global",
  global_path: `/tmp/${id}.yaml`,
  local_path: "",
  global_exists: true,
  local_exists: false,
});

const registry = {
  modes: [
    configItem(
      "modes",
      "very_long_mode_name_for_responsive_testing",
      "Very Long Mode Name For Responsive Testing",
    ),
  ],
  subagents: [
    configItem(
      "subagents",
      "responsive_subagent_with_long_identifier",
      "Responsive Subagent With Long Identifier",
    ),
  ],
  toolbox_commands: [
    configItem(
      "toolbox_commands",
      "long_toolbox_command_name_that_should_truncate",
      "Long Toolbox Command Name That Should Truncate",
    ),
  ],
  code_lens: [
    configItem(
      "code_lens",
      "long_code_lens_action_name",
      "Long Code Lens Action",
    ),
  ],
  errors: [],
  has_project_root: true,
};

const integrations: IntegrationWithIconResponse = {
  integrations: [
    {
      project_path: "",
      integr_name: "github_enterprise_with_a_very_long_name",
      icon_path: "/integrations/github.svg",
      integr_config_path: ".config/refact/integrations/github_enterprise.yaml",
      integr_config_exists: true,
      on_your_laptop: true,
      when_isolated: true,
    },
    {
      project_path: "/workspace/very/long/project/path/for/responsive/testing",
      integr_name: "mcp_TEMPLATE",
      icon_path: "/integrations/mcp.svg",
      integr_config_path: "/workspace/.refact/integrations/mcp_TEMPLATE.yaml",
      integr_config_exists: false,
      on_your_laptop: true,
      when_isolated: true,
    },
  ],
  error_log: [],
};

const extensions: ExtRegistryResponse = {
  skills: [
    {
      name: "responsive-skill-with-a-very-long-name",
      description:
        "A long skill description that should wrap inside the settings hub.",
      source: "/tmp/skills/responsive.md",
      source_label: "global",
      scope: "global",
      read_only: false,
      file_path: "/tmp/skills/responsive.md",
    },
  ],
  slash_commands: [
    {
      name: "responsive-command-with-a-very-long-name",
      description:
        "A long command description that should wrap inside the settings hub.",
      source: "/tmp/commands/responsive.md",
      source_label: "global",
      scope: "global",
      read_only: false,
      file_path: "/tmp/commands/responsive.md",
    },
  ],
  hooks: [
    {
      event: "on_session_start_with_long_event_name",
      command: "echo responsive settings hook",
      source: "/tmp/hooks.yaml",
      source_label: "global",
      scope: "global",
      read_only: false,
    },
  ],
  has_project_root: true,
};

const marketplaceSources = [
  {
    id: "refact-starter",
    label: "Refact Starter Source With Long Name",
    description: "Bundled marketplace entries for responsive testing.",
    enabled: true,
    builtin: true,
    removable: false,
    source_kind: "builtin_embedded",
    supported_kinds: ["skill", "command", "subagent"],
    parser_mode: "scan",
    item_count: 3,
  },
];

const extensionMarketplaceItem = (kind: "skill" | "command" | "subagent") => ({
  id: `responsive-${kind}`,
  name: `Responsive ${kind} marketplace item with long name`,
  description:
    "A marketplace item with enough descriptive text to prove the toolbar and cards wrap cleanly on narrow screens.",
  tags: ["responsive", "layout", "marketplace"],
  publisher: "Refact",
  kind,
  source_id: "refact-starter",
  source_label: "Refact Starter Source With Long Name",
  path: `${kind}s/responsive`,
  installed_scopes: [],
});

const mcpMarketplace = {
  servers: [
    {
      id: "responsive-mcp-server",
      source_id: "refact-bundled",
      name: "Responsive MCP Server With Long Name",
      description:
        "A representative MCP server card used by the route showcase responsiveness check.",
      publisher: "Refact",
      tags: ["responsive", "tools", "marketplace"],
      transport: "stdio",
      install_recipe: { command: "npx responsive-mcp" },
      confirmation_default: [],
    },
  ],
  sources: [
    {
      id: "refact-bundled",
      label: "Refact Bundled MCP Registry With Long Name",
      type: "refact_index",
      enabled: true,
      removable: false,
      server_count: 1,
      status: "ok",
    },
  ],
  pagination: { page: 1, page_size: 20, total: 1 },
};

const pluginMarketplaces = {
  marketplaces: [
    {
      name: "responsive-plugins",
      source: "JegernOUTT/refact-plugins-responsive-fixture",
      added_at: now,
    },
  ],
};

const pluginList = {
  plugins: [
    {
      name: "responsive-plugin-with-a-very-long-name",
      description:
        "A plugin marketplace item with long text for the marketplace route showcase.",
      version: "1.0.0",
      tags: ["responsive", "plugin"],
      marketplace: "responsive-plugins",
    },
  ],
};

const cronTasks = [
  {
    id: "responsive-cron-task",
    cron: "*/15 * * * *",
    human_schedule: "Every fifteen minutes with a long schedule description",
    description:
      "Responsive scheduler task with a deliberately long description",
    prompt: "Check responsive settings state",
    recurring: true,
    durable: true,
    next_fire_at_ms: Date.now() + 900000,
    fire_count: 42,
    created_at_ms: Date.now() - 3600000,
  },
];

function jsonResponse(data: unknown) {
  return new Response(JSON.stringify(data), {
    headers: { "Content-Type": "application/json" },
  });
}

const nativeFetch = window.fetch.bind(window);
const showcaseCommandLog: unknown[] = [];
const showcaseClipboardWrites: string[] = [];
(
  window as unknown as { __routeShowcaseCommands: unknown[] }
).__routeShowcaseCommands = showcaseCommandLog;
(
  window as unknown as { __routeShowcaseClipboardWrites: string[] }
).__routeShowcaseClipboardWrites = showcaseClipboardWrites;
Object.defineProperty(window.navigator, "clipboard", {
  configurable: true,
  value: {
    writeText: (text: string) => {
      showcaseClipboardWrites.push(text);
      return Promise.resolve();
    },
  },
});
window.fetch = (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.toString()
        : input.url,
    window.location.origin,
  );
  const path = url.pathname;
  if (path.startsWith("/v1/chats/")) {
    if (init?.body) {
      try {
        showcaseCommandLog.push(JSON.parse(String(init.body)));
      } catch {
        showcaseCommandLog.push(init.body);
      }
    }
    return Promise.resolve(jsonResponse({ ok: true }));
  }
  if (path === "/v1/buddy") return Promise.resolve(jsonResponse(buddySnapshot));
  if (path === "/v1/buddy/settings") {
    if (init?.method === "POST" && init.body) {
      const patch = JSON.parse(String(init.body)) as Partial<BuddySettings>;
      return Promise.resolve(jsonResponse({ ...buddySettings, ...patch }));
    }
    return Promise.resolve(jsonResponse(buddySettings));
  }
  if (path === "/v1/buddy/opportunities") {
    return Promise.resolve(jsonResponse({ opportunities: [buddyOpportunity] }));
  }
  if (path === "/v1/buddy/conversations") {
    if (init?.method === "POST") {
      return Promise.resolve(
        jsonResponse({
          chat_id: "showcase-new-buddy-chat",
          title: "New Buddy showcase chat",
          created_at: now,
        }),
      );
    }
    return Promise.resolve(jsonResponse(buddyConversations));
  }
  if (path === "/v1/buddy/pulse")
    return Promise.resolve(jsonResponse(buddyPulse));
  if (path.startsWith("/v1/buddy/runtime/")) {
    return Promise.resolve(jsonResponse({ dismissed: true }));
  }
  if (path.startsWith("/v1/buddy/opportunities/")) {
    return Promise.resolve(
      jsonResponse({
        snapshot: buddySnapshot,
        action_result: { kind: "dismiss" },
      }),
    );
  }
  if (path.startsWith("/v1/buddy/care")) {
    return Promise.resolve(
      jsonResponse({ message: "ok", snapshot: buddySnapshot }),
    );
  }
  if (path === "/v1/ping") return Promise.resolve(new Response("pong"));
  if (path === "/v1/setup/status") {
    return Promise.resolve(
      jsonResponse({
        configured: true,
        reasons: [],
        detail: {
          project_root: "/workspace/refact",
          has_agents_md: true,
          has_knowledge: true,
          has_trajectories: true,
        },
      }),
    );
  }
  if (path === "/v1/stats/llm/summary") {
    return Promise.resolve(
      jsonResponse({
        date_range: { from: "2026-06-01", to: "2026-06-07" },
        totals: {
          total_calls: 42,
          successful_calls: 39,
          failed_calls: 3,
          total_prompt_tokens: 12000,
          total_completion_tokens: 4000,
          total_tokens: 16000,
          total_cache_read_tokens: 0,
          total_cache_creation_tokens: 0,
          total_cost_usd: 0.12,
          total_duration_ms: 42000,
          avg_duration_ms: 1000,
          total_conversations: 8,
          total_messages_sent: 55,
        },
        by_model: [],
        by_provider: [],
        by_day: [],
        by_mode: [],
        top_conversations: [],
      }),
    );
  }
  if (path === "/v1/providers")
    return Promise.resolve(jsonResponse(providerList));
  if (path === "/v1/chat-modes")
    return Promise.resolve(jsonResponse(chatModesResponse));
  if (path === "/v1/caps") return Promise.resolve(jsonResponse(caps));
  if (path === "/v1/defaults") return Promise.resolve(jsonResponse(defaults));
  if (path === "/v1/customization/registry") {
    return Promise.resolve(jsonResponse(registry));
  }
  if (path === "/v1/integrations")
    return Promise.resolve(jsonResponse(integrations));
  if (path === "/v1/scheduler/cron")
    return Promise.resolve(jsonResponse(cronTasks));
  if (path === "/v1/docs-list") {
    return Promise.resolve(
      jsonResponse([
        {
          url: "https://docs.example.com/a/very/long/documentation/source/url",
          max_depth: 3,
          max_pages: 25,
          pages: { index: "Index", quickstart: "Quickstart" },
        },
      ]),
    );
  }
  if (path === "/v1/ext/registry")
    return Promise.resolve(jsonResponse(extensions));
  if (path === "/v1/skills/marketplace") {
    return Promise.resolve(
      jsonResponse({
        items: [extensionMarketplaceItem("skill")],
        sources: marketplaceSources,
      }),
    );
  }
  if (path === "/v1/commands/marketplace") {
    return Promise.resolve(
      jsonResponse({
        items: [extensionMarketplaceItem("command")],
        sources: marketplaceSources,
      }),
    );
  }
  if (path === "/v1/subagents/marketplace") {
    return Promise.resolve(
      jsonResponse({
        items: [extensionMarketplaceItem("subagent")],
        sources: marketplaceSources,
      }),
    );
  }
  if (path === "/v1/mcp/marketplace")
    return Promise.resolve(jsonResponse(mcpMarketplace));
  if (path === "/v1/mcp/marketplace/installed")
    return Promise.resolve(jsonResponse({ installed: [] }));
  if (path === "/v1/plugins/marketplaces")
    return Promise.resolve(jsonResponse(pluginMarketplaces));
  if (path === "/v1/plugins/installed")
    return Promise.resolve(jsonResponse({ installed: [] }));
  if (path === "/v1/plugins/marketplace/responsive-plugins/plugins")
    return Promise.resolve(jsonResponse(pluginList));
  return nativeFetch(input, init);
};

const preloadedState: Partial<RootState> = {
  config: {
    host: "web",
    lspPort: 8001,
    lspUrl: "",
    dev: true,
    engineServed: false,
    apiKey: null,
    features: { statistics: true, vecdb: true, ast: true, images: true },
    themeProps: { appearance: "dark" },
    shiftEnterToSubmit: false,
  },
  chat: {
    current_thread_id: chatId,
    open_thread_ids: [chatId],
    threads: { [chatId]: chatRuntime },
    max_new_tokens: 4096,
    tool_use: "agent",
    system_prompt: {},
    sse_refresh_requested: null,
    stream_version: 0,
  },
  history: {
    chats: {
      alpha: historyItem("alpha", "Dashboard route smoke coverage", now),
      beta: historyItem(
        "beta",
        "Chat surface responsive check",
        "2026-06-06T10:00:00Z",
      ),
    },
    isLoading: false,
    loadError: null,
    pagination: { cursor: null, hasMore: false, totalCount: 2, generation: 1 },
  },
  pages: [{ name: "history" }],
};

const store = setUpStore(preloadedState);

store.dispatch(setBackendStatus({ status: "online" }));
for (const section of ["workspace", "chats", "tasks", "buddy"] as const) {
  store.dispatch(sidebarSectionSnapshotReceived({ section, status: "ready" }));
}
store.dispatch(setBuddySnapshot(buddySnapshot));
store.dispatch(tasksApi.util.upsertQueryData("listTasks", undefined, tasks));

const route =
  new URLSearchParams(window.location.search).get("route") ?? "dashboard";
const root = document.getElementById("refact-chat");

if (!root) {
  throw new Error("Missing #refact-chat route showcase root");
}

const OverlayRegressionSurface = () => {
  const [model, setModel] = React.useState("openai_codex_personal/gpt-5.5");
  const [mode, setMode] = React.useState("agent");

  return (
    <div
      style={{
        display: "grid",
        gap: "var(--rf-space-4)",
        justifyItems: "start",
        minWidth: 0,
        padding: "var(--rf-space-5)",
      }}
    >
      <TrajectoryButton />
      <ModelSelector
        groups={overlayRegressionModelGroups}
        models={overlayRegressionModels}
        value={model}
        onSelect={setModel}
      />
      <ModeSelect selectedMode={mode} onModeChange={setMode} />
    </div>
  );
};

const ShowcaseSurface = () => {
  const currentPage = useSelector((state: RootState) => {
    const page = state.pages[state.pages.length - 1];
    return page && isMarketplacePage(page)
      ? page
      : marketplaceTabToPage(marketplaceTab);
  });

  if (route === "overlay-regression") {
    return <OverlayRegressionSurface />;
  }

  if (route === "chat") {
    return <Chat host="web" tabbed={false} backFromChat={() => undefined} />;
  }

  if (route === "settings") {
    return (
      <SettingsHub
        page={settingsSectionToPage(settingsSection)}
        onBack={() => undefined}
        host="web"
        tabbed={false}
      />
    );
  }

  if (route === "marketplace") {
    return (
      <MarketplaceHub
        page={currentPage}
        back={() => undefined}
        host="web"
        tabbed={false}
      />
    );
  }

  if (route === "buddy") {
    return <BuddyHome />;
  }

  return <Dashboard />;
};

const Showcase = () => {
  return (
    <Provider store={store}>
      <Theme>
        <AbortControllerProvider>
          <Flex
            align="stretch"
            direction="column"
            data-element="app-root"
            style={{
              width: "100%",
              maxWidth: "100%",
              minWidth: 0,
              height: "100vh",
              overflow: "hidden",
            }}
          >
            <ShowcaseSurface />
          </Flex>
        </AbortControllerProvider>
      </Theme>
    </Provider>
  );
};

createRoot(root).render(<Showcase />);
