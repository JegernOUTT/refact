import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { Flex } from "@radix-ui/themes";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { ToolContent } from "../../src/components/ChatContent/ToolsContent";
import { CollapsibleStoreProvider } from "../../src/components/ChatContent/useStoredOpen";
import {
  makeChatSlice,
  makeChatThread,
} from "../../src/__stories__/chatStoryState";
import { COMPACT_AGENTIC_MESSAGES } from "../../src/__fixtures__/toolCardsAgentic";
import {
  launchPreviewCall,
  launchPreviewReceipt,
} from "../../src/components/Tools/AgenticTool/launchFixtures";
import { applyChatEvent } from "../../src/features/Chat/Thread/actions";
import type {
  BackgroundAgentSummary,
  PendingQuestionSet,
  ToolCall,
  ToolMessage,
} from "../../src/services/refact/types";
import "../../src/lib/render/web.css";

const params = new URLSearchParams(window.location.search);
const host = params.get("host");
const threadId = "compact-transcript";
const calls: ToolCall[] = COMPACT_AGENTIC_MESSAGES.flatMap((message) =>
  message.role === "assistant" ? message.tool_calls ?? [] : [],
);
const results: ToolMessage[] = COMPACT_AGENTIC_MESSAGES.filter(
  (message): message is ToolMessage => message.role === "tool",
);
results.push({
  role: "tool",
  tool_call_id: "compact-unknown",
  content: "Authority outcome was not recorded",
});

function add(
  id: string,
  name: string,
  args: Record<string, unknown>,
  content?: string,
  extra?: ToolMessage["extra"],
) {
  calls.push({
    id,
    index: calls.length,
    type: "function",
    function: { name, arguments: JSON.stringify(args) },
  });
  if (content !== undefined)
    results.push({ role: "tool", tool_call_id: id, content, extra });
}

const background: BackgroundAgentSummary = {
  agent_id: "bg-browser",
  parent_chat_id: threadId,
  child_chat_id: "browser-child",
  kind: "subagent",
  status: "running",
  title: "Trace the parser boundary",
  progress: "Reading the parser",
  step_count: 2,
  last_activity: null,
  target_files: ["src/parser.ts"],
  edited_files: [],
  diff_summary: null,
  conflict_summary: null,
  result_summary: null,
  error: null,
  started_at: null,
  finished_at: null,
  change_seq: 1,
  model: "local/fixture",
  current_tool: "cat",
};
add(
  "compact-background",
  "subagent",
  { task: background.title },
  "Launch accepted",
  {
    background_agent_id: background.agent_id,
    background_agent_kind: background.kind,
    background_agent_status: background.status,
    child_chat_id: background.child_chat_id,
  },
);
calls.push(launchPreviewCall);
results.push(launchPreviewReceipt());
add(
  "compact-status",
  "check_agents",
  {},
  JSON.stringify({
    agents: [
      {
        card_id: "T-14",
        title: "Parser inspection",
        state: "running",
        priority: "P1",
      },
    ],
  }),
);
add(
  "compact-pulse",
  "agent_pulse",
  { card_id: "T-14" },
  [
    "# Agent Pulse: T-14",
    "**Card:** Parser inspection",
    "**State:** paused",
    "**Last activity:** 3m ago",
    "**Currently editing:** src/parser.ts",
    "## Last assistant message",
    "> Awaiting review",
    "## Last tool call",
    "`cat src/parser.ts`",
  ].join("\n"),
);
add(
  "compact-diff",
  "agent_diff",
  { card_id: "T-14" },
  [
    "# Agent Diff for T-14",
    "**Card:** Parser inspection",
    "**Branch:** refact/parser",
    "**Base:** commit abc123",
    "```diff",
    "diff --git a/src/parser.ts b/src/parser.ts",
    "--- a/src/parser.ts",
    "+++ b/src/parser.ts",
    "@@ -1 +1 @@",
    "-old parser",
    "+new parser",
    "```",
  ].join("\n"),
);
add(
  "compact-documents",
  "doc_list",
  {},
  [
    "| slug | name | kind | pinned | version | updated_at |",
    "|---|---|---|---|---|---|",
    "| parser-spec | Parser specification | spec | true | 2 | 2026-10-25 |",
  ].join("\n"),
);
add(
  "compact-document",
  "doc_get",
  { slug: "parser-spec" },
  [
    "---",
    "name: Parser specification",
    "slug: parser-spec",
    "kind: spec",
    "version: 2",
    "---",
    "# Parser boundary",
    "Preserve the complete document body.",
  ].join("\n"),
);
add(
  "compact-paused",
  "get_goal",
  {},
  JSON.stringify({
    goal: {
      content: "Preserve the parser boundary",
      status: "paused",
      version: 1,
      delta_count: 0,
      turns_used: 2,
      tokens_used: 300,
      latest_verdict: null,
      gaps: [],
    },
  }),
);
add("compact-board", "board_get", {}, "No recognized board receipt");
add(
  "compact-review",
  "code_review",
  { files: ["src/parser.ts"] },
  "Review stage unavailable",
);
add(
  "compact-legacy-launch",
  "subagent",
  { task: "Inspect archived output" },
  "Legacy child report; lifecycle unavailable",
);
add(
  "compact-task-done",
  "task_done",
  {},
  JSON.stringify({
    type: "task_done",
    summary: "Parser implementation reported",
    report: "## Delivery\nImplementation claimed; no checks supplied.",
    files_changed: ["src/parser.ts"],
  }),
);
add("compact-awaiting", "agent_finish", {});
add("compact-question", "ask_questions", {});
const pending: PendingQuestionSet = {
  tool_call_id: "compact-question",
  asked_at_ms: 42,
  response_protocol_version: 2,
  status: "pending",
  questions: [
    {
      id: "scope",
      type: "single_select",
      text: "Which parser scope?",
      options: ["Parser only", "Include lexer"],
    },
  ],
};
const store = setUpStore({
  config: {
    host: host === "vscode" || host === "jetbrains" ? host : "web",
    lspPort: 8001,
    lspUrl: window.location.origin,
    dev: true,
    apiKey: null,
    themeProps: {
      appearance: params.get("theme") === "dark" ? "dark" : "light",
    },
  },
  chat: makeChatSlice(
    makeChatThread({
      id: threadId,
      messages: [
        {
          role: "assistant",
          content: "Inspect compact receipts",
          tool_calls: calls,
        },
        ...results,
      ],
      pending_questions: [pending],
    }),
    {
      background_agents: { [background.agent_id]: background },
      streaming: params.has("busy"),
    },
  ),
});
const openChoices = new Map<string, boolean>();
const collapseStore = {
  get: (key: string) => openChoices.get(key),
  set: (key: string, value: boolean) => {
    openChoices.set(key, value);
  },
};

function Fixture() {
  const [mounted, setMounted] = React.useState(true);
  const [updated, setUpdated] = React.useState(false);
  return (
    <Provider store={store}>
      <Theme>
        <AbortControllerProvider>
          <CollapsibleStoreProvider value={collapseStore}>
            <main aria-label="Compact production transcript">
              <Flex gap="2" wrap="wrap">
                <button onClick={() => setMounted((value) => !value)}>
                  {mounted ? "Unmount transcript" : "Mount transcript"}
                </button>
                <button
                  disabled={updated}
                  onClick={() => {
                    store.dispatch(
                      applyChatEvent({
                        type: "background_agent_updated",
                        chat_id: threadId,
                        seq: "1",
                        agent: {
                          ...background,
                          status: "completed",
                          result_summary:
                            "Parser inspected; verification pending",
                          change_seq: 2,
                        },
                      }),
                    );
                    setUpdated(true);
                  }}
                >
                  Receive child update
                </button>
              </Flex>
              {mounted &&
                calls.map((call) => (
                  <article key={call.id} data-testid={call.id}>
                    <ToolContent toolCalls={[call]} threadId={threadId} />
                  </article>
                ))}
              <button>After transcript</button>
            </main>
          </CollapsibleStoreProvider>
        </AbortControllerProvider>
      </Theme>
    </Provider>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Compact transcript root is missing");
createRoot(root).render(<Fixture />);
