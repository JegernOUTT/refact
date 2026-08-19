import type { ChatMessages, ToolCall } from "../services/refact";

type FixtureToolCall = {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
};

type FixtureToolResult = {
  id: string;
  content: string;
  extra?: Record<string, unknown>;
};

function toolCall(
  { id, name, arguments: args }: FixtureToolCall,
  index: number,
): ToolCall {
  return {
    id,
    function: {
      name,
      arguments: JSON.stringify(args),
    },
    type: "function",
    index,
  };
}

function conversation(
  request: string,
  calls: FixtureToolCall[],
  results: FixtureToolResult[],
  summary: string,
): ChatMessages {
  return [
    { role: "user", content: request },
    {
      role: "assistant",
      content: "",
      tool_calls: calls.map(toolCall),
    },
    ...results.map(({ id, content, extra }) => ({
      role: "tool" as const,
      tool_call_id: id,
      content,
      tool_failed: false,
      ...(extra ? { extra } : {}),
    })),
    { role: "assistant", content: summary },
  ];
}

export const CAT_TOOL_MESSAGES = conversation(
  "Read the app entry point and its configuration.",
  [
    {
      id: "fileops-cat",
      name: "cat",
      arguments: { paths: "src/main.ts, src/config.ts" },
    },
  ],
  [
    {
      id: "fileops-cat",
      content: `Paths found:

src/main.ts
1: import { startServer } from "./server";
2: import { config } from "./config";
3:
4: startServer(config.port);

src/config.ts
1: export const config = {
2:   port: 4173,
3:   logLevel: "info",
4: } as const;`,
    },
  ],
  "The app starts the server on port 4173 with info-level logging.",
);

export const TREE_TOOL_MESSAGES = conversation(
  "Show me the project layout.",
  [
    {
      id: "fileops-tree",
      name: "tree",
      arguments: { path: ".", use_ast: false, max_files: 30 },
    },
  ],
  [
    {
      id: "fileops-tree",
      content: `.
├── package.json
├── README.md
├── src
│   ├── config.ts
│   ├── main.ts
│   ├── server.ts
│   └── routes
│       ├── health.ts
│       └── users.ts
└── tests
    ├── health.test.ts
    └── users.test.ts`,
    },
  ],
  "This is a small TypeScript service with route modules and matching tests.",
);

export const REGEX_SEARCH_TOOL_MESSAGES = conversation(
  "Find every TODO in the source tree.",
  [
    {
      id: "fileops-regex",
      name: "regex_search",
      arguments: { pattern: "TODO|FIXME", scope: "src" },
    },
  ],
  [
    {
      id: "fileops-regex",
      content: `src/server.ts:18:  // TODO: add graceful shutdown
src/routes/users.ts:42:  // FIXME: paginate database results
src/config.ts:9:  // TODO: validate environment overrides`,
    },
  ],
  "There are three follow-ups: shutdown handling, pagination, and config validation.",
);

export const SHELL_TOOL_MESSAGES = conversation(
  "Print the runtime versions used by this project.",
  [
    {
      id: "fileops-shell",
      name: "shell",
      arguments: {
        command: "node --version && npm --version",
        workdir: "/workspace/refact-demo",
        description: "Check JavaScript runtime versions",
      },
    },
  ],
  [
    {
      id: "fileops-shell",
      content: `stdout:
v22.11.0
10.9.0

stderr:
<empty>`,
      extra: {
        exec: {
          process_id: "proc-runtime-versions",
          status: "exited",
          command: "node --version && npm --version",
          cwd: "/workspace/refact-demo",
          short_description: "Check JavaScript runtime versions",
          started_at_ms: 1735689600000,
          ended_at_ms: 1735689600240,
          duration_ms: 240,
          exit_code: 0,
          transcript: {
            process_id: "proc-runtime-versions",
            found: true,
            latest_seq: 1,
            chunks: [
              {
                process_id: "proc-runtime-versions",
                seq: 1,
                stream: "stdout",
                text: "v22.11.0\n10.9.0\n",
                timestamp_ms: 1735689600200,
              },
            ],
            total_lines_appended: 2,
          },
        },
      },
    },
  ],
  "The project is using Node.js 22.11.0 and npm 10.9.0.",
);

export const MOVE_REMOVE_TOOL_MESSAGES = conversation(
  "Rename the draft config and remove its obsolete backup.",
  [
    {
      id: "fileops-mv",
      name: "mv",
      arguments: {
        source: "config/app.draft.json",
        destination: "config/app.json",
      },
    },
    {
      id: "fileops-rm",
      name: "rm",
      arguments: { path: "config/app.json.bak", recursive: false },
    },
  ],
  [
    {
      id: "fileops-mv",
      content: `Moved config/app.draft.json
   to config/app.json`,
    },
    {
      id: "fileops-rm",
      content: `Removed config/app.json.bak
1 file deleted`,
    },
  ],
  "The draft is now the active config, and the stale backup was removed.",
);

export const WEB_TOOL_MESSAGES = conversation(
  "Fetch the release notes page and summarize the latest release.",
  [
    {
      id: "fileops-web",
      name: "web",
      arguments: { url: "https://example.com/releases/2.4" },
    },
  ],
  [
    {
      id: "fileops-web",
      content: `# Refact Demo 2.4

Released January 14, 2025.

## Highlights
- Faster workspace indexing.
- Clearer tool execution summaries.
- Improved recovery after interrupted tasks.

Upgrading does not require a configuration migration.`,
    },
  ],
  "Version 2.4 improves indexing, tool summaries, and recovery without requiring migration.",
);

export const WEB_SEARCH_TOOL_MESSAGES = conversation(
  "Search the web for TypeScript 5.7 release information.",
  [
    {
      id: "fileops-web-search",
      name: "web_search",
      arguments: { query: "TypeScript 5.7 release notes", num_results: 3 },
    },
  ],
  [
    {
      id: "fileops-web-search",
      content: `1. [Announcing TypeScript 5.7](https://devblogs.microsoft.com/typescript/announcing-typescript-5-7/)
   Official announcement covering checks for never-initialized variables and path rewriting.

2. [TypeScript 5.7 Release Notes](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-5-7.html)
   Handbook documentation with examples of the new compiler behavior.

3. [TypeScript 5.7 on npm](https://www.npmjs.com/package/typescript/v/5.7.2)
   Package metadata and installation details for the stable release.`,
    },
  ],
  "The official announcement and handbook provide the most complete TypeScript 5.7 details.",
);

export const KNOWLEDGE_TOOL_MESSAGES = conversation(
  "Recall our deployment convention, then save the new rollback note.",
  [
    {
      id: "fileops-knowledge",
      name: "knowledge",
      arguments: { search_key: "production deployment convention" },
    },
    {
      id: "fileops-save-knowledge",
      name: "save_knowledge",
      arguments: {
        content:
          "Production rollback: redeploy the previous immutable image tag; never rebuild an old commit.",
      },
    },
  ],
  [
    {
      id: "fileops-knowledge",
      content: `Memory matches:
- Production deploys use immutable image tags of the form release-YYYY.MM.DD.N.
- The deploy workflow requires a green smoke test before traffic promotion.
- Database migrations must remain backward compatible for one release.`,
    },
    {
      id: "fileops-save-knowledge",
      content: `Knowledge saved successfully.
Key: production-rollback-immutable-image
Scope: workspace`,
    },
  ],
  "I recalled the production conventions and saved the immutable-image rollback rule.",
);

export const GENERIC_FALLBACK_TOOL_MESSAGES = conversation(
  "Ask the experimental tool to inspect the build signal.",
  [
    {
      id: "fileops-mystery",
      name: "mystery_tool",
      arguments: { signal: "build-health", depth: 2, include_history: true },
    },
  ],
  [
    {
      id: "fileops-mystery",
      content: `Build signal: healthy
Recent runs: 12 passed, 0 failed
Median duration: 3m 18s
Observation: cache hit rate increased to 87%.`,
    },
  ],
  "The experimental check reports a healthy build signal and improved cache usage.",
);

export const MANY_TOOLS_GROUPED_MESSAGES = conversation(
  "Quickly inspect the project, locate the server entry, and check its runtime.",
  [
    {
      id: "fileops-group-tree",
      name: "tree",
      arguments: { path: "src", max_files: 20 },
    },
    {
      id: "fileops-group-regex",
      name: "regex_search",
      arguments: { pattern: "startServer", scope: "src" },
    },
    {
      id: "fileops-group-cat",
      name: "cat",
      arguments: { paths: "src/main.ts" },
    },
    {
      id: "fileops-group-shell",
      name: "shell",
      arguments: {
        command: "node --version",
        workdir: "/workspace/refact-demo",
      },
    },
  ],
  [
    {
      id: "fileops-group-tree",
      content: `src
├── main.ts
├── server.ts
└── config.ts`,
    },
    {
      id: "fileops-group-regex",
      content: `src/main.ts:1:import { startServer } from "./server";
src/main.ts:4:startServer(config.port);`,
    },
    {
      id: "fileops-group-cat",
      content: `src/main.ts
1: import { startServer } from "./server";
2: import { config } from "./config";
3:
4: startServer(config.port);`,
    },
    {
      id: "fileops-group-shell",
      content: `stdout:
v22.11.0

stderr:
<empty>`,
      extra: {
        exec: {
          process_id: "proc-group-node-version",
          status: "exited",
          command: "node --version",
          cwd: "/workspace/refact-demo",
          exit_code: 0,
          duration_ms: 84,
        },
      },
    },
  ],
  "The entry point is src/main.ts, which starts the server under Node.js 22.11.0.",
);
