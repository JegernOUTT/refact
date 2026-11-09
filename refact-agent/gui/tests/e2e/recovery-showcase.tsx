import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { ChatContent } from "../../src/components/ChatContent/ChatContent";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { ChatThreadProvider } from "../../src/features/Chat/Thread";
import {
  applyChatEvent,
  createChatWithId,
} from "../../src/features/Chat/Thread/actions";
import { setThemeMode } from "../../src/features/Config/configSlice";
import { selectRecoveryById } from "../../src/features/Chat/Thread/selectors";
import type {
  ChatEventEnvelope,
  RecoveryRecord,
} from "../../src/services/refact/chatSubscription";
import { Button, FieldText } from "../../src/components/ui";
import { useChatActions } from "../../src/hooks/useChatActions";
import styles from "./recovery-showcase.module.css";

const chatId = "recovery-showcase";
const store = setUpStore();
store.dispatch(createChatWithId({ id: chatId }));
let seq = 1;
const timeout = "LLM HTTP SSE response header timeout after 30s";
const transport =
  "LLM request failed: error sending request for url (https://example.test/responses)";
const base: RecoveryRecord = {
  operation_id: "operation",
  run_id: "run-1",
  revision: 5,
  state: "exhausted",
  policy: {
    mode: "transient",
    max_retries: 2,
    delay_ms: 10000,
    total_timeout_ms: 600000,
  },
  policy_source: "project",
  manual: false,
  retries_used: 2,
  terminal_reason: "retry_limit_exhausted",
  latest_error: {
    category: "ProviderTransient",
    title: "Provider temporarily unavailable",
    explanation: "The model provider returned a temporary error.",
    suggested_action: "retry",
    is_retryable: true,
    raw_error: timeout,
  },
  attempts: [
    {
      at_ms: 1000,
      retry_number: 0,
      outcome: "failed",
      error: {
        category: "NetworkFailure",
        title: "Network failure",
        explanation: "Connection failed.",
        suggested_action: "retry",
        is_retryable: true,
        raw_error: transport,
      },
    },
    {
      at_ms: 2000,
      retry_number: 1,
      outcome: "failed",
      error: {
        category: "ProviderTransient",
        title: "Timeout",
        explanation: "Timed out.",
        suggested_action: "retry",
        is_retryable: true,
        raw_error: timeout,
      },
    },
  ],
};

function event(value: Omit<ChatEventEnvelope, "seq" | "chat_id">) {
  store.dispatch(
    applyChatEvent({
      ...value,
      chat_id: chatId,
      seq: String(seq++),
    } as ChatEventEnvelope),
  );
}

const scenarios = [
  "exhausted",
  "waiting",
  "attempting",
  "needs_action",
  "cancelled",
  "succeeded",
  "privacy",
  "cooldown",
  "goal-held",
  "queued",
  "request",
  "request-no-retry",
  "multiple",
  "unbroken",
  "multiline",
] as const;
type Scenario = (typeof scenarios)[number];
const params = new URLSearchParams(window.location.search);
const initialScenario =
  scenarios.find((value) => value === params.get("scenario")) ?? "exhausted";
const commandMode = params.get("command") ?? "applied";
const privacyError =
  "Privacy policy blocked sending this request to Example provider. Protected project files and conversation content cannot be shared with this destination. Review the privacy policy and choose an approved provider before continuing. No private content was sent.";
const unbrokenError = `Provider request failed: ${"unbroken_request_identifier_".repeat(
  18,
)}`;
const multilineError =
  "Provider request failed\nThe connection closed before a response arrived.\nCheck the connection and retry when ready.";

function loadScenario(scenario: Scenario) {
  const standalone = scenario === "request" || scenario === "request-no-retry";
  const state: RecoveryRecord["state"] = scenarios
    .slice(0, 6)
    .includes(scenario)
    ? (scenario as RecoveryRecord["state"])
    : scenario === "privacy"
      ? "needs_action"
      : scenario === "cooldown"
        ? "waiting"
        : "exhausted";
  const rawError =
    scenario === "privacy"
      ? privacyError
      : scenario === "unbroken"
        ? unbrokenError
        : scenario === "multiline"
          ? multilineError
          : timeout;
  const record: RecoveryRecord = {
    ...base,
    state,
    terminal_reason: state === "exhausted" ? base.terminal_reason : null,
    latest_error: { ...base.latest_error, raw_error: rawError },
    attempts:
      scenario === "multiple" || scenario === "exhausted" ? base.attempts : [],
    retry_not_before_ms: scenario === "cooldown" ? Date.now() + 60000 : null,
    run_id: `run-${seq}`,
    retries_used: state === "waiting" ? 0 : base.retries_used,
    next_retry_at_ms: state === "waiting" ? Date.now() + 60000 : null,
    deadline_at_ms: Date.now() + 600000,
  };
  const archived = {
    role: "error" as const,
    message_id: "archive",
    content: base.latest_error.title,
    recovery_record: { ...base, run_id: "previous-run" },
    error_info: { ...base.latest_error, raw_error: base.latest_error.title },
    _ui_only: true,
  };
  event({
    type: "snapshot",
    thread: {
      id: chatId,
      title: "Recovery showcase",
      model: "example/model",
      mode: "agent",
      tool_use: "agent",
      boost_reasoning: false,
      context_tokens_cap: 8192,
      include_project_info: false,
      checkpoints_enabled: false,
      is_title_generated: false,
    },
    runtime: {
      state: "error",
      paused: false,
      error: "Provider temporarily unavailable",
      queue_size: scenario === "queued" ? 1 : 0,
      queued_items:
        scenario === "queued"
          ? [
              {
                client_request_id: "queued-retry",
                command_type: "recovery_retry",
                queued_at_ms: Date.now(),
              },
            ]
          : [],
      pause_reasons: [],
      recovery: standalone ? null : record,
    },
    goal:
      scenario === "goal-held"
        ? {
            status: "paused",
            goal: "Review remaining sample outputs",
            stop_reason: "recovery_exhausted",
          }
        : null,
    background_agents: [],
    messages: [
      {
        role: "user",
        message_id: "user",
        content: "Review the remaining sample outputs.",
      },
      {
        role: "assistant",
        message_id: "assistant",
        content:
          "I’ll inspect the remaining comparison images before continuing.",
      },
      {
        role: "event",
        message_id: "goal-notice",
        subkind: "system_notice",
        source: "goal",
        content: "Goal paused for provider recovery",
      },
      {
        role: "assistant",
        message_id: "tool-only",
        content: " \n ",
        tool_calls: [
          {
            id: "goal-tool",
            type: "function",
            function: {
              name: "goal",
              arguments: JSON.stringify({ action: "status" }),
            },
          },
          {
            id: "subagent-tool",
            type: "function",
            function: {
              name: "subagent",
              arguments: JSON.stringify({ action: "list" }),
            },
          },
        ],
      },
      ...(standalone
        ? [{ role: "error", message_id: "standalone", content: multilineError }]
        : scenario === "exhausted" || scenario === "multiple"
          ? [archived]
          : []),
    ],
  } as Omit<ChatEventEnvelope, "seq" | "chat_id">);
}

loadScenario(initialScenario);

window.fetch = async (input, init) => {
  const url =
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.href
        : input.url;
  const json = (value: unknown) =>
    Promise.resolve(
      new Response(JSON.stringify(value), {
        headers: { "Content-Type": "application/json" },
      }),
    );
  if (init?.method === "DELETE") {
    if (commandMode === "cancelling")
      return new Promise<Response>(() => undefined);
    if (commandMode === "cancel-error")
      throw new TypeError("Connection interrupted");
  }
  if (url.includes("/commands") && init?.method === "POST") {
    const request = JSON.parse(String(init.body)) as {
      type: string;
      client_request_id: string;
      content?: string;
      patch?: Record<string, unknown>;
    };
    if (
      (request.type === "abort" && commandMode === "stop-error") ||
      (request.type === "goal_control" && commandMode === "resume-error")
    ) {
      throw new TypeError("Connection interrupted");
    }
    if (request.type.startsWith("recovery_retry")) {
      if (commandMode === "sending")
        return new Promise<Response>(() => undefined);
      if (commandMode === "uncertain")
        throw new TypeError("Connection interrupted");
      if (commandMode === "request-error")
        return new Response(
          JSON.stringify({ detail: "Retry rejected by provider policy" }),
          { status: 409, headers: { "Content-Type": "application/json" } },
        );
    }
    event({
      type: "ack",
      client_request_id: request.client_request_id,
      accepted: true,
    } as Omit<ChatEventEnvelope, "seq" | "chat_id">);
    if (request.type === "recovery_retry" && commandMode !== "pending") {
      const current = selectRecoveryById(store.getState(), chatId) ?? base;
      event({
        type: "recovery_updated",
        recovery: {
          ...current,
          run_id: `manual-${seq}`,
          revision: current.revision + 1,
          manual: true,
          retries_used: 0,
          state: "attempting",
          policy: { ...current.policy, max_retries: 1 },
        },
      } as Omit<ChatEventEnvelope, "seq" | "chat_id">);
    } else if (request.type === "user_message") {
      event({
        type: "message_added",
        message: {
          role: "user",
          message_id: `user-${seq}`,
          content: request.content ?? "Continue",
        },
      } as Omit<ChatEventEnvelope, "seq" | "chat_id">);
    } else if (request.type === "set_params") {
      event({ type: "thread_updated", params: request.patch ?? {} } as Omit<
        ChatEventEnvelope,
        "seq" | "chat_id"
      >);
      event({
        type: "ack",
        client_request_id: request.client_request_id,
        accepted: true,
        result: { applied: true },
      } as Omit<ChatEventEnvelope, "seq" | "chat_id">);
    }
    return json({ accepted: true });
  }
  if (url.includes("coordination"))
    return json({
      effective: {
        recovery_auto_retry_enabled: true,
        recovery_max_retries: 2,
        recovery_delay_ms: 10000,
        recovery_total_timeout_ms: 600000,
      },
      inherited: [
        "recovery_auto_retry_enabled",
        "recovery_max_retries",
        "recovery_delay_ms",
        "recovery_total_timeout_ms",
      ],
    });
  if (url.includes("privacy/inspect"))
    return json({
      chat_id: chatId,
      destination: { kind: "provider", id: "example", display_name: "Example" },
      sendable: true,
      records: [],
      blocked: [],
      refusal: null,
    });
  if (url.includes("privacy/policy"))
    return json({
      destinations: [],
      policy: {
        blocked: [],
        zones: [],
        subagents: { report_declassifies: false },
        tool_access: { providers: {} },
      },
      source_paths: [],
      match_counts: {},
      error: null,
      has_project_overrides: false,
    });
  if (url.includes("/caps"))
    return json({
      cloud_name: "Preview",
      chat_models: {},
      code_completion_models: {},
      embedding_models: {},
      tokenizer_rewrite_path: {},
      running_models: [],
      models_dict_patch: {},
    });
  return json({});
};

function Showcase() {
  const [draft, setDraft] = useState("");
  const [scenario, setScenario] = useState<Scenario>(initialScenario);
  const [theme, setTheme] = useState<"light" | "dark">("dark");
  const actions = useChatActions(chatId);
  return (
    <main className={styles.page}>
      <div className={styles.toolbar}>
        <label>
          State{" "}
          <select
            aria-label="Scenario"
            value={scenario}
            onChange={(e) => {
              const next = e.target.value as Scenario;
              setScenario(next);
              loadScenario(next);
            }}
          >
            {scenarios.map((state) => (
              <option key={state}>{state}</option>
            ))}
          </select>
        </label>
        <Button
          size="sm"
          onClick={() => {
            const next = theme === "dark" ? "light" : "dark";
            setTheme(next);
            store.dispatch(setThemeMode(next));
          }}
        >
          Toggle theme
        </Button>
      </div>
      <div className={styles.chat}>
        <ChatContent
          onRetry={() => undefined}
          onStopStreaming={() => undefined}
          onRetryGeneration={
            scenario === "request-no-retry" ? undefined : () => undefined
          }
        />
      </div>
      <form
        className={styles.composer}
        onSubmit={(e) => {
          e.preventDefault();
          void actions.submit(draft).then(() => setDraft(""));
        }}
      >
        <FieldText aria-label="Message" value={draft} onChange={setDraft} />
        <Button type="submit">Send</Button>
      </form>
    </main>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Missing recovery showcase root");
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <AbortControllerProvider>
        <ChatThreadProvider chatId={chatId}>
          <Showcase />
        </ChatThreadProvider>
      </AbortControllerProvider>
    </Theme>
  </Provider>,
);
