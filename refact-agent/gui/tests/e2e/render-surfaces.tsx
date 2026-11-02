import "../../src/lib/index";
import "../../src/lib/render/web.css";
import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { ChatThreadProvider } from "../../src/features/Chat/Thread";
import {
  makeChatSlice,
  makeChatThread,
} from "../../src/__stories__/chatStoryState";
import { STUB_CAPS_RESPONSE } from "../../src/__fixtures__/caps";
import { ChatForm } from "../../src/components/ChatForm/ChatForm";
import { RecoveryCard } from "../../src/components/ChatContent/RecoveryCard";
import { TaskProgressWidget } from "../../src/components/TaskProgressWidget";
import { ActivityRail } from "../../src/features/Workspace/ActivityRail/ActivityRail";
import { GitDock } from "../../src/features/Workspace/GitPanel/GitDock";
import type { RecoveryRecord } from "../../src/services/refact/chatSubscription";
import type {
  ChatMessages,
  GoalSnapshot,
} from "../../src/services/refact/types";
import type {
  GitChange,
  GitStatusRoot,
} from "../../src/services/refact/gitRead";
import styles from "./render-surfaces.module.css";

const params = new URLSearchParams(location.search);
const view = params.get("view") ?? "composer";
const active = params.has("busy");
const threadId = "render-surface-chat";
const model = "openai_codex_personal/large-reasoning-model-with-long-name";
const now = Date.parse("2026-11-01T00:00:00Z");
const branch = "feature/long-branch-name-for-responsive-controls";
const longText =
  "Preserve all original observations, unchanged geometry and approved acceptance criteria. ";
const tasks = [
  "Pin the current baseline and define the exact bilateral producer-to-consumer contract",
  "Implement exact bilateral ingress, continuous certification and role-owned objective primitives",
  "Run deterministic checks and independent adversarial reviews of the new primitives",
  "Validate the real pre-composition producer capture and exact slicing without shrinking original geometry",
  "Integrate the complete solve and versioned consumers, then repeat the regeneration/recheck loop",
].map((content, index) => ({
  id: String(index),
  content,
  status: index < 3 ? "completed" : index === 3 ? "in_progress" : "pending",
}));
const goal: GoalSnapshot = {
  content: `## Validate the complete producer-to-consumer contract\n\n${longText.repeat(
    60,
  )}\n\n${"original_observations_".repeat(35)}`,
  version: 3,
  active: true,
  status: "active",
  budget: { cooldown_ms: 1500, no_progress_token_threshold: 300 },
  progress: {
    turns_used: 83,
    tokens_used: 654640,
    started_at_ms: now,
    no_progress_turns: 0,
  },
  attempts: Array.from({ length: 10 }, (_, index) => ({
    at_ms: now + index,
    trigger: "checkpoint",
    verdict: "needs_work",
    gaps: [longText.repeat(3)],
    verifier_reply: longText.repeat(8),
  })),
  events: Array.from({ length: 30 }, (_, index) => ({
    at_ms: now + index,
    kind: "goal_delta",
    text: `Approved candidate ${index}: ${longText.repeat(10)}`,
  })),
  transferred_from: null,
  transferred_to: null,
  plan: {
    content: `## Approved execution plan\n\n${longText.repeat(30)}`,
    mode: "agent",
    version: 2,
    created_at_ms: now,
    delta_count: 2,
    checkpoint: null,
    history: [
      { seq: 1, content: "Initial approved plan boundary" },
      { seq: 2, content: "Additional verification boundary" },
    ],
  },
};
const recoveryState = params.get("state");
const recovery: RecoveryRecord = {
  operation_id: "render-operation",
  run_id: "render-run",
  revision: 1,
  state:
    recoveryState === "succeeded"
      ? "succeeded"
      : recoveryState === "waiting"
        ? "waiting"
        : "exhausted",
  policy: {
    mode: "transient",
    max_retries: 3,
    delay_ms: 1500,
    total_timeout_ms: 30000,
  },
  policy_source: "thread",
  manual: false,
  retries_used: 1,
  latest_error: {
    category: "StreamCorrupted",
    title: "Stream corrupted",
    explanation:
      "The provider stream ended unexpectedly or returned data that could not be decoded safely.",
    suggested_action: "retry",
    is_retryable: true,
    raw_error: `Connection reset\nhttps://provider.example.com/${"unbroken_provider_segment_".repeat(
      40,
    )}\nLast diagnostic line`,
  },
  next_retry_at_ms: Date.now() + 60000,
  deadline_at_ms: Date.now() + 120000,
  attempts: Array.from(
    { length: params.has("history") ? 100 : 2 },
    (_, index) => ({
      at_ms: now + index,
      retry_number: index,
      outcome: index === 0 ? "failed" : "succeeded",
    }),
  ),
};
const changeCount = params.has("virtual")
  ? 240
  : params.has("huge")
    ? 10000
    : 25;
const changes: GitChange[] = Array.from({ length: changeCount }, (_, index) => {
  const path = `railgeom/branch_${
    index % 12
  }/deep_${index}/observations_${"long_name_".repeat(12)}_${index}.py`;
  return {
    relative_path: path,
    absolute_path: `/workspace/refact/${path}`,
    old_path: null,
    status: "MODIFIED",
    staged: false,
    unstaged: true,
  };
});
const gitRoot: GitStatusRoot = {
  root: "/workspace/refact",
  branch,
  head_detached: false,
  ahead: 3,
  behind: 1,
  staged: [],
  unstaged: changes,
  untracked_included: true,
  changes,
  untracked: [],
  conflicted: [],
  truncated: true,
  repo_state: "clean",
};
const commands: unknown[] = [];
Object.assign(window, { __renderSurfaceCommands: commands });
const nativeFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = new URL(
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input
        : input.url,
    location.origin,
  );
  const path = url.pathname;
  if (!path.startsWith("/v1/")) return nativeFetch(input, init);
  const json = (value: unknown) =>
    new Response(JSON.stringify(value), {
      headers: { "Content-Type": "application/json" },
    });
  if (path === "/v1/ping") return new Response("pong");
  if (path.includes(`/chats/${threadId}/commands`)) {
    const body = init?.body
      ? String(init.body)
      : input instanceof Request
        ? await input.clone().text()
        : "";
    commands.push(body ? JSON.parse(body) : null);
    return json({ ok: true });
  }
  if (path === "/v1/caps")
    return json({
      ...STUB_CAPS_RESPONSE,
      chat_default_model: model,
      chat_models: {
        [model]: {
          ...STUB_CAPS_RESPONSE.chat_models["gpt-4o"],
          name: model,
          id: model,
          n_ctx: 872000,
          supports_multimodality: true,
          supports_tools: true,
          supports_agent: true,
        },
      },
    });
  if (path === "/v1/providers")
    return json({
      providers: [
        {
          name: "openai_codex_personal",
          base_provider: "openai",
          display_name: "OpenAI Codex Personal With A Very Long Display Name",
          enabled: true,
          readonly: false,
          has_credentials: true,
          status: "active",
          model_count: 1,
        },
      ],
      error_log: [],
    });
  if (path.endsWith("/quota"))
    return json({
      quota: {
        provider_name: "openai_codex_personal",
        base_provider: "openai",
        source: "fixture",
        fetched_at: new Date(now).toISOString(),
        available: true,
        stale: false,
        windows: [{ id: "daily", label: "Daily usage", used_percent: 42 }],
        facts: [],
      },
    });
  if (path === "/v1/chat-modes")
    return json({
      modes: [
        {
          id: "agent",
          title: "Agent",
          description: "Autonomous coding",
          tools_count: 86,
          thread_defaults: {
            include_project_info: true,
            checkpoints_enabled: true,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
          },
          ui: { order: 1, tags: ["editing"] },
        },
        {
          id: "ask",
          title: "Ask",
          description: "Discuss code",
          tools_count: 0,
          thread_defaults: {
            include_project_info: true,
            checkpoints_enabled: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
          },
          ui: { order: 2, tags: [] },
        },
      ],
      errors: [],
    });
  if (path === "/v1/worktrees")
    return json({
      project_hash: "render-surfaces",
      source_workspace_root: "/workspace/refact",
      source_current_branch: branch,
      source_branches: [branch, "main"],
      worktrees: [],
    });
  if (path === "/v1/git/status") return json({ roots: [gitRoot] });
  if (path === "/v1/git/log")
    return json({ roots: [{ root: gitRoot.root, commits: [] }] });
  if (path === "/v1/git/branches")
    return json({
      roots: [
        {
          root: gitRoot.root,
          current: branch,
          branches: [],
          remotes: [],
          remote_branches: [],
          tags: [],
        },
      ],
    });
  if (path === "/v1/voice/status")
    return json({
      enabled: false,
      model_loaded: false,
      model_name: "",
      is_downloading: false,
      download_progress: 0,
    });
  if (path === "/v1/at-command-completion")
    return json({ completions: [], replace: [0, 0], is_cmd_executable: false });
  if (path === "/v1/at-command-preview") return json({ messages: [] });
  if (path === "/v1/tools") return json([]);
  if (path === "/v1/links")
    return json({ links: [], uncommited_changes_warning: "" });
  if (path === "/v1/customization") return json({ system_prompts: {} });
  if (path === "/v1/privacy/policy")
    return json({
      policy: {
        blocked: [],
        zones: [],
        subagents: { report_declassifies: true },
        tool_access: { providers: {} },
      },
      destinations: [],
      match_counts: {},
      error: null,
      source_paths: [],
      has_project_overrides: false,
    });
  if (path === "/v1/privacy/inspect")
    return json({
      chat_id: threadId,
      destination: { id: model, kind: "provider", display_name: model },
      sendable: true,
      records: [],
      blocked: [],
      refusal: null,
    });
  return json({});
};
const messages: ChatMessages = [
  {
    role: "assistant",
    content: "Tracking the approved work.",
    usage: {
      prompt_tokens: params.has("context-warning") ? 793520 : 654640,
      completion_tokens: 83,
      total_tokens: params.has("context-warning") ? 793603 : 654723,
      ...(params.has("paid")
        ? {
            metering_usd: { prompt_usd: 120, generated_usd: 3, total_usd: 123 },
          }
        : {}),
    },
    tool_calls: [
      {
        id: "render-tasks",
        index: 0,
        type: "function",
        function: { name: "tasks_set", arguments: JSON.stringify({ tasks }) },
      },
    ],
  },
  {
    role: "tool",
    tool_call_id: "render-tasks",
    content: "Task list updated",
    tool_failed: false,
  },
];
const thread = makeChatThread({
  id: threadId,
  model,
  mode: "agent",
  tool_use: "agent",
  messages,
  ...(view === "goal" || view === "combined" ? { goal } : {}),
});
const store = setUpStore({
  config: {
    host: params.get("host") === "jetbrains" ? "jetbrains" : "web",
    lspPort: 8001,
    lspUrl: location.origin,
    dev: true,
    apiKey: null,
    themeProps: {
      appearance: params.get("theme") === "light" ? "light" : "dark",
    },
    features: { images: true, vecdb: true, ast: true },
  },
  connection: {
    browserOnline: true,
    backendStatus: "online",
    backendLastOkAt: Date.now(),
    backendError: null,
    sseConnections: {},
    visibleChatMounts: {},
    suspended: false,
  },
  chat: makeChatSlice(thread, {
    streaming: active,
    session_state: active ? "generating" : "idle",
    task_widget_expanded: params.has("expanded"),
    task_goal_expanded: params.has("expanded"),
    ...(view === "recovery" ? { recovery } : {}),
  }),
});
function Surface() {
  if (view === "git")
    return (
      <main className={styles.git} aria-label="Git render fixture">
        <ActivityRail />
        <div className={styles.gitPanel}>
          <GitDock />
        </div>
      </main>
    );
  return (
    <main className={styles.page} aria-label={`${view} render fixture`}>
      {view === "recovery" ? (
        <RecoveryCard chatId={threadId} recovery={recovery} />
      ) : null}
      {view === "goal" || view === "setup" ? <TaskProgressWidget /> : null}
      {view === "composer" || view === "combined" ? (
        <div className={styles.dock}>
          {view === "combined" ? <TaskProgressWidget /> : null}
          <ChatForm
            embedded
            onSubmit={() => undefined}
            onClose={() => undefined}
          />
        </div>
      ) : null}
      <button type="button" className={styles.outside}>
        Outside composer
      </button>
    </main>
  );
}
const root = document.getElementById("root");
if (!root) throw new Error("Render fixture root missing");
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <AbortControllerProvider>
        <ChatThreadProvider chatId={threadId}>
          <Surface />
        </ChatThreadProvider>
      </AbortControllerProvider>
    </Theme>
  </Provider>,
);
