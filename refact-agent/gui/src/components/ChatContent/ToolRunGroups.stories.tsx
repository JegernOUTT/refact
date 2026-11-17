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
