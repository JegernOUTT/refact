import React from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import {
  CHAT_STORY_MSW_HANDLERS,
  type ThreadRuntime,
} from "../../__stories__/chatStoryState";
import type { ChatMessages, DiffChunk, ToolCall } from "../../services/refact";
import { ChatContent } from "./ChatContent";

const T0 = 1_763_400_000_000;

type CallSpec = {
  id: string;
  name: string;
  args: Record<string, unknown>;
  at?: [number, number];
};

function call({ id, name, args, at }: CallSpec, index: number): ToolCall {
  return {
    id,
    index,
    type: "function",
    function: { name, arguments: JSON.stringify(args) },
    ...(at ? { started_at_ms: T0 + at[0], completed_at_ms: T0 + at[1] } : {}),
  };
}

function calls(specs: CallSpec[]): ToolCall[] {
  return specs.map(call);
}

function result(
  id: string,
  content: string,
  extra?: Record<string, unknown>,
  failed = false,
): ChatMessages[number] {
  return {
    role: "tool",
    tool_call_id: id,
    content,
    tool_failed: failed,
    ...(extra ? { extra } : {}),
  };
}

function chunk(file: string, add: string, remove: string): DiffChunk {
  return {
    file_name: file,
    file_action: "edit",
    line1: 10,
    line2: 12,
    lines_add: add,
    lines_remove: remove,
  };
}

const EXPLORATION: ChatMessages = [
  {
    role: "user",
    content: "Why does the dev server start on the wrong port?",
    message_id: "x-user",
  },
  {
    role: "assistant",
    content: "",
    message_id: "x-a1",
    reasoning_content:
      "I should look at the project layout, then find where the port is configured.",
    tool_calls: calls([
      { id: "x-tree", name: "tree", args: { path: "src" }, at: [0, 600] },
      {
        id: "x-search",
        name: "search_pattern",
        args: { pattern: "port", scope: "src/" },
        at: [700, 1_900],
      },
      {
        id: "x-cat",
        name: "cat",
        args: { paths: "src/config.ts, src/server.ts" },
        at: [2_000, 2_400],
      },
      {
        id: "x-web",
        name: "web",
        args: { url: "https://vitejs.dev/config/server-options" },
        at: [2_500, 9_800],
      },
      {
        id: "x-shell",
        name: "shell",
        args: { command: "cat .env | grep PORT", description: "Read PORT" },
        at: [9_900, 11_200],
      },
    ]),
  },
  result("x-tree", "src/\n  config.ts\n  server.ts\n  main.ts"),
  result("x-search", "src/config.ts:2:  port: Number(process.env.PORT)"),
  result("x-cat", "src/config.ts\n1: export const config = {}"),
  result("x-web", "server.port: Specify server port."),
  result("x-shell", "PORT=4173"),
  {
    role: "assistant",
    content:
      "The port comes from `PORT` in `.env` (4173), which overrides the Vite default of 5173.",
    message_id: "x-a2",
  },
];

const EDITS: ChatMessages = [
  {
    role: "user",
    content: "Make the port configurable and add a test.",
    message_id: "e-user",
  },
  {
    role: "assistant",
    content: "I'll update the config and the server, then run the tests.",
    message_id: "e-a1",
    tool_calls: calls([
      {
        id: "e-edit1",
        name: "update_textdoc",
        args: { path: "src/config.ts" },
        at: [0, 900],
      },
      {
        id: "e-edit2",
        name: "update_textdoc",
        args: { path: "src/server.ts" },
        at: [1_000, 2_100],
      },
      {
        id: "e-test",
        name: "shell",
        args: { command: "npm test", description: "Run unit tests" },
        at: [2_200, 96_000],
      },
      {
        id: "e-edit3",
        name: "update_textdoc",
        args: { path: "src/config.ts" },
        at: [96_500, 97_200],
      },
      {
        id: "e-test2",
        name: "shell",
        args: { command: "npm test", description: "Run unit tests" },
        at: [97_300, 134_000],
      },
    ]),
  },
  result("e-edit1", "ok"),
  {
    role: "diff",
    content: [
      chunk(
        "src/config.ts",
        "  port: Number(process.env.PORT ?? 5173),\n  host: process.env.HOST,\n",
        "  port: 4173,\n",
      ),
    ],
    tool_call_id: "e-edit1",
    message_id: "e-d1",
  },
  result("e-edit2", "ok"),
  {
    role: "diff",
    content: [
      chunk(
        "src/server.ts",
        "startServer(config.port, config.host);\n",
        "startServer(4173);\n",
      ),
    ],
    tool_call_id: "e-edit2",
    message_id: "e-d2",
  },
  result("e-test", "FAIL src/config.test.ts\n  expected 5173", undefined, true),
  result("e-edit3", "ok"),
  {
    role: "diff",
    content: [
      chunk("src/config.ts", "export const DEFAULT_PORT = 5173;\n", ""),
    ],
    tool_call_id: "e-edit3",
    message_id: "e-d3",
  },
  result("e-test2", "PASS src/config.test.ts"),
  {
    role: "assistant",
    content:
      "Done: the port and host are read from the environment, with 5173 as the default. Tests pass.",
    message_id: "e-a2",
  },
];

const SINGLE: ChatMessages = [
  { role: "user", content: "What is in main.ts?", message_id: "s-user" },
  {
    role: "assistant",
    content: "",
    message_id: "s-a1",
    tool_calls: calls([
      {
        id: "s-cat",
        name: "cat",
        args: { paths: "src/main.ts" },
        at: [0, 300],
      },
    ]),
  },
  result("s-cat", "src/main.ts\n1: startServer(config.port);"),
  {
    role: "assistant",
    content: "`main.ts` only starts the server with the configured port.",
    message_id: "s-a2",
  },
];

const LIVE_TAIL: ChatMessages = [
  { role: "user", content: "Check the build.", message_id: "l-user" },
  {
    role: "assistant",
    content: "Let me look at the build configuration first.",
    message_id: "l-a1",
    tool_calls: calls([
      {
        id: "l-cat",
        name: "cat",
        args: { paths: "vite.config.ts" },
        at: [0, 400],
      },
      {
        id: "l-shell",
        name: "shell",
        args: { command: "npm run build", description: "Build" },
      },
    ]),
  },
  result("l-cat", "vite.config.ts\n1: export default defineConfig({})"),
];

const ATTENTION: ChatMessages = [
  {
    role: "user",
    content: "Start the dev server and explore.",
    message_id: "n-user",
  },
  {
    role: "assistant",
    content: "",
    message_id: "n-a1",
    tool_calls: calls([
      {
        id: "n-cat",
        name: "cat",
        args: { paths: "package.json" },
        at: [0, 200],
      },
      {
        id: "n-dev",
        name: "shell",
        args: {
          command: "npm run dev",
          description: "Start dev server",
          run_in_background: true,
        },
        at: [300, 1_100],
      },
      {
        id: "n-tree",
        name: "tree",
        args: { path: "src" },
        at: [1_200, 1_500],
      },
    ]),
  },
  result("n-cat", '{ "scripts": { "dev": "vite" } }'),
  result("n-dev", "Process started in background", {
    exec: {
      process_id: "proc-dev",
      status: "running",
      mode: "background",
      command: "npm run dev",
      short_description: "Start dev server",
      started_at_ms: T0 + 300,
    },
  }),
  result("n-tree", "src/\n  main.ts"),
  {
    role: "assistant",
    content: "The dev server is running in the background on port 5173.",
    message_id: "n-a2",
  },
];

const OLD_HISTORY: ChatMessages = [
  { role: "user", content: "Find the TODOs.", message_id: "o-user" },
  {
    role: "assistant",
    content: "",
    message_id: "o-a1",
    tool_calls: calls([
      { id: "o-search", name: "search_pattern", args: { pattern: "TODO" } },
      { id: "o-glob", name: "glob", args: { pattern: "**/*.md" } },
      { id: "o-cat", name: "cat", args: { paths: "README.md" } },
    ]),
  },
  result("o-search", "src/server.ts:4: // TODO graceful shutdown"),
  result("o-glob", "README.md"),
  result("o-cat", "README.md\n1: # App"),
  {
    role: "assistant",
    content: "There is one TODO: graceful shutdown in `src/server.ts`.",
    message_id: "o-a2",
  },
];

const MULTI_STEP: ChatMessages = [
  { role: "user", content: "Tidy up the server module.", message_id: "m-user" },
  {
    role: "assistant",
    content: "",
    message_id: "m-a1",
    tool_calls: calls([
      {
        id: "m-cat",
        name: "cat",
        args: { paths: "src/server.ts" },
        at: [0, 300],
      },
    ]),
  },
  result("m-cat", "src/server.ts\n1: startServer(4173);"),
  {
    role: "assistant",
    content: "",
    message_id: "m-a2",
    tool_calls: calls([
      {
        id: "m-search",
        name: "search_pattern",
        args: { pattern: "startServer" },
        at: [400, 900],
      },
      {
        id: "m-glob",
        name: "glob",
        args: { pattern: "src/**/*.test.ts" },
        at: [950, 1_200],
      },
    ]),
  },
  result("m-search", "src/server.ts:1: startServer(4173);"),
  result("m-glob", "src/server.test.ts"),
  {
    role: "assistant",
    content: "",
    message_id: "m-a3",
    tool_calls: calls([
      {
        id: "m-shell",
        name: "shell",
        args: { command: "npm run lint", description: "Lint" },
        at: [1_300, 4_800],
      },
    ]),
  },
  result("m-shell", "0 problems"),
  {
    role: "assistant",
    content: "The server module is clean; lint reports no problems.",
    message_id: "m-a4",
  },
];

function notice(
  id: string,
  content: string,
  source = "agents.push",
  subkind = "system_notice",
): ChatMessages[number] {
  return {
    role: "event",
    message_id: id,
    subkind,
    source,
    content,
  };
}

function lifecycleReceipt(
  agentId: string,
  status: string,
  accepted: boolean,
): string {
  return JSON.stringify({
    action: "continue",
    accepted,
    agent_id: agentId,
    child_chat_id: null,
    status,
    change_seq: 4,
    run_generation: 1,
    step_count: 12,
    cap_stop: null,
    can_resume: false,
    max_steps: null,
    resume_configuration_available: true,
    original_scope_available: true,
  });
}

const SCRIPT_CHECKS = [
  "Count migrated invoices",
  "Compare refund totals",
  "Check currency rounding",
  "Verify ledger balances",
  "Scan for orphaned rows",
  "Diff API fixtures",
  "Check UI snapshot list",
  "Validate webhook payloads",
  "Measure migration time",
  "Check index usage",
  "Summarize agent diffs",
];

const SCRIPT_CALLS: CallSpec[] = SCRIPT_CHECKS.map((description, index) => ({
  id: `b-py-${index}`,
  name: "python",
  args: { description, code: `# ${description}\nprint("ok")` },
  at: [40_000 + index * 2_000, 41_500 + index * 2_000],
}));

const BUDDY: ChatMessages = [
  {
    role: "user",
    content: "Ship the billing migration with the helper agents.",
    message_id: "b-user",
  },
  notice("b-n1", "agent bgagent-3f2a: schema migration drafted"),
  {
    role: "assistant",
    content: "",
    message_id: "b-a1",
    tool_calls: calls([
      {
        id: "b-status",
        name: "shell",
        args: { command: "git status --short", description: "Check tree" },
        at: [0, 400],
      },
      { id: "b-list", name: "agent_list", args: {}, at: [500, 700] },
      {
        id: "b-overview",
        name: "agents_overview",
        args: {},
        at: [800, 1_100],
      },
    ]),
  },
  result("b-status", " M src/billing/schema.sql"),
  result("b-list", "bgagent-3f2a running\nbgagent-91c4 running"),
  result(
    "b-overview",
    [
      "- bgagent-3f2a — Schema migration — ✅ completed — step 12",
      "- bgagent-91c4 — API rounding fix — 🔄 running — step 8",
      "- bgagent-5d07 — UI update — 🔄 running — step 5",
    ].join("\n"),
  ),
  notice("b-n2", "agent bgagent-91c4: API tests failing on currency rounding"),
  notice(
    "b-q1",
    "[subagent question] agent bgagent-91c4 asks: should refunds round half-even?",
  ),
  {
    role: "assistant",
    content: "",
    message_id: "b-thought",
    reasoning_content:
      "The API agent is blocked on rounding. Half-even matches the ledger, so answer it and let it continue.",
  },
  {
    role: "assistant",
    content: "",
    message_id: "b-a2",
    tool_calls: calls([
      {
        id: "b-message",
        name: "agent_message",
        args: {
          to: "bgagent-91c4",
          text: "Use half-even rounding to match the ledger.",
        },
        at: [9_000, 9_300],
      },
    ]),
  },
  result("b-message", "Message queued for bgagent-91c4"),
  notice(
    "b-approval",
    "User approved 1 tool call (once)",
    "agents.approval",
    "tool_decision",
  ),
  {
    role: "assistant",
    content: "",
    message_id: "b-a3",
    tool_calls: calls([
      {
        id: "b-life-1",
        name: "agent_lifecycle",
        args: { agent_id: "bgagent-5d07", action: "continue" },
        at: [10_000, 10_400],
      },
      {
        id: "b-life-2",
        name: "agent_lifecycle",
        args: { agent_id: "bgagent-91c4", action: "continue" },
        at: [10_500, 10_900],
      },
    ]),
  },
  result("b-life-1", lifecycleReceipt("bgagent-5d07", "failed", false)),
  result("b-life-2", lifecycleReceipt("bgagent-91c4", "running", true)),
  notice(
    "b-goal",
    "Goal pursuit nudge: 2 of 3 agents are still running",
    "chat.goal_monitor",
  ),
  {
    role: "assistant",
    content: "",
    message_id: "b-a4",
    tool_calls: calls([
      {
        id: "b-wait-1",
        name: "wait",
        args: { target: { kind: "agents", agent_ids: ["bgagent-91c4"] } },
        at: [11_000, 24_000],
      },
    ]),
  },
  result(
    "b-wait-1",
    JSON.stringify({ target: "agents", outcome: "interrupted" }),
  ),
  notice("b-n3", "agent bgagent-5d07: UI components updated"),
  {
    role: "assistant",
    content: "",
    message_id: "b-a5",
    tool_calls: calls([
      {
        id: "b-wait-2",
        name: "wait",
        args: { target: { kind: "agents", agent_ids: ["bgagent-91c4"] } },
        at: [24_500, 39_000],
      },
    ]),
  },
  result(
    "b-wait-2",
    JSON.stringify({ target: "agents", outcome: "interrupted" }),
  ),
  {
    role: "assistant",
    content: "",
    message_id: "b-a6",
    tool_calls: calls(SCRIPT_CALLS),
  },
  ...SCRIPT_CALLS.map((spec) => result(spec.id, "ok")),
  notice("b-n4", "agent bgagent-3f2a: finished"),
  {
    role: "assistant",
    content:
      "All three agents finished: the schema migration, the half-even refund rounding fix and the UI update are merged, and the verification scripts pass.",
    message_id: "b-final",
  },
];

const CARDS: ChatMessages = [
  {
    role: "user",
    content: "Refresh the billing context and summarize what changed.",
    message_id: "c-user",
  },
  {
    role: "assistant",
    content: "",
    message_id: "c-a1",
    tool_calls: calls([
      {
        id: "c-cat",
        name: "cat",
        args: { paths: "src/billing/refunds.ts" },
        at: [0, 300],
      },
    ]),
  },
  result("c-cat", "src/billing/refunds.ts\n1: export const refunds = {}"),
  {
    role: "context_file",
    message_id: "c-memories",
    tool_call_id: "knowledge_enrichment",
    content: [
      {
        file_name: "2026-11-01_120000_a1b2c3_refund-rounding.md",
        file_content: "Refunds round half-even to match the ledger.",
        line1: 1,
        line2: 1,
      },
    ],
  },
  {
    role: "summarization",
    message_id: "c-summary",
    content: "Earlier turns about the schema migration were summarized.",
    summarization_tier: "tier1_llm",
    summarized_token_estimate: 1800,
  },
  {
    role: "cd_instruction",
    message_id: "c-skill",
    content:
      '💿 SKILL_ACTIVATED {"name":"billing-review","allowed_tools":["cat"]}\nReview billing changes before merging.',
  },
  {
    role: "event",
    message_id: "tool_job:c-job:1:0",
    subkind: "system_notice",
    source: "tool_job.completed",
    content: "code_health: billing module health 8.4/10",
    extra: {
      event: {
        subkind: "system_notice",
        source: "tool_job.completed",
        payload: { actor: "tool_completion", job_id: "c-job" },
      },
      tool_job_result: {
        actor: "tool_completion",
        job_id: "c-job",
        revision: 1,
        tool_name: "code_health",
        tool_call_id: "c-health",
        original_role: "tool",
      },
      delivery: { source: "tool_job.completed", id: "tool_job:c-job:1" },
    },
  },
  {
    role: "diff",
    content: [
      chunk(
        "src/billing/refunds.ts",
        "  rounding: 'half-even',\n",
        "  rounding: 'half-up',\n",
      ),
    ],
    tool_call_id: "c-external",
    message_id: "c-diff",
  },
  {
    role: "assistant",
    content:
      "Refunds now round half-even everywhere; the earlier schema discussion is summarized above.",
    message_id: "c-final",
  },
];

type ToolRunStoryProps = {
  messages: ChatMessages;
  runtime?: Partial<ThreadRuntime>;
};

const ToolRunStory: React.FC<ToolRunStoryProps> = ({ messages, runtime }) => (
  <ChatStoryHarness messages={messages} runtime={runtime}>
    <ChatContent onRetry={() => ({})} onStopStreaming={() => ({})} />
  </ChatStoryHarness>
);

const meta = {
  title: "Chat Content/Tool Run Groups",
  component: ToolRunStory,
  parameters: {
    msw: {
      handlers: [...CHAT_STORY_MSW_HANDLERS],
    },
  },
} satisfies Meta<typeof ToolRunStory>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Exploration: Story = {
  args: { messages: EXPLORATION },
};

export const EditsWithFailure: Story = {
  args: { messages: EDITS },
};

export const SingleTool: Story = {
  args: { messages: SINGLE },
};

export const LiveTail: Story = {
  args: {
    messages: LIVE_TAIL,
    runtime: { streaming: true, waiting_for_response: false },
  },
};

export const PinnedAttention: Story = {
  args: { messages: ATTENTION },
};

export const OldHistory: Story = {
  args: { messages: OLD_HISTORY },
};

export const MultiStep: Story = {
  args: { messages: MULTI_STEP },
};

export const BuddyNotices: Story = {
  args: { messages: BUDDY },
};

export const FoldedCards: Story = {
  args: { messages: CARDS },
};

export const AllCases: Story = {
  args: {
    messages: [
      ...EXPLORATION,
      ...EDITS,
      ...SINGLE,
      ...ATTENTION,
      ...OLD_HISTORY,
      ...MULTI_STEP,
    ],
  },
};
