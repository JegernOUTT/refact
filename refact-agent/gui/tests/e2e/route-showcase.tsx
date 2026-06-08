import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { Flex } from "@radix-ui/themes";

import { setUpStore, type RootState } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { Dashboard } from "../../src/features/Dashboard";
import { Chat } from "../../src/features/Chat";
import {
  SETTINGS_SECTIONS,
  SettingsHub,
  settingsSectionToPage,
  type SettingsSectionId,
} from "../../src/features/Settings";
import { setBackendStatus } from "../../src/features/Connection";
import { sidebarSectionSnapshotReceived } from "../../src/features/Sidebar/sidebarSlice";
import { tasksApi, type TaskMeta } from "../../src/services/refact/tasks";
import type { ChatHistoryItem } from "../../src/features/History/historySlice";
import type { ChatThreadRuntime } from "../../src/features/Chat/Thread";
import type { ChatMessages } from "../../src/services/refact";
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
          arguments: JSON.stringify({ paths: "src/components/ui/ToolCard.tsx" }),
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
    content: "Process started\nstdout:\nroute showcase ready\nstderr:\n<empty>\n",
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
(window as unknown as { __routeShowcaseCommands: unknown[] }).__routeShowcaseCommands =
  showcaseCommandLog;
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
  if (path === "/v1/ping") return Promise.resolve(new Response("pong"));
  if (path === "/v1/providers")
    return Promise.resolve(jsonResponse(providerList));
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
store.dispatch(tasksApi.util.upsertQueryData("listTasks", undefined, tasks));

const route =
  new URLSearchParams(window.location.search).get("route") ?? "dashboard";
const root = document.getElementById("refact-chat");

if (!root) {
  throw new Error("Missing #refact-chat route showcase root");
}

const Showcase = () => {
  const surface = (() => {
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

    return <Dashboard />;
  })();

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
            {surface}
          </Flex>
        </AbortControllerProvider>
      </Theme>
    </Provider>
  );
};

createRoot(root).render(<Showcase />);
