import { describe, expect, it } from "vitest";
import type {
  AssistantMessage,
  ChatMessages,
  DiffChunk,
  EventMessage,
  ToolCall,
  ToolResult,
} from "../../services/refact";
import { buildDisplayItems, type DisplayItem } from "./ChatContentDisplayItems";
import {
  formatToolRunDuration,
  groupToolRuns,
  summarizeToolRun,
  toolRunCallState,
  toolRunDurationMs,
  toolRunSummaryText,
  type ToolRunCallState,
  type TranscriptDisplayItem,
} from "./toolRunGroups";

const NO_STATES: ReadonlyMap<string, ToolRunCallState> = new Map();

let callSeq = 0;

function call(
  name: string,
  args: Record<string, unknown> = {},
  stamps?: [number, number],
): ToolCall {
  callSeq += 1;
  return {
    id: `call-${callSeq}`,
    index: 0,
    type: "function",
    function: { name, arguments: JSON.stringify(args) },
    ...(stamps ? { started_at_ms: stamps[0], completed_at_ms: stamps[1] } : {}),
  };
}

function assistant(
  id: string,
  content: string,
  toolCalls: ToolCall[] = [],
  extra: Partial<AssistantMessage> = {},
): AssistantMessage {
  return {
    role: "assistant",
    message_id: id,
    content,
    ...(toolCalls.length > 0 ? { tool_calls: toolCalls } : {}),
    ...extra,
  };
}

function results(calls: ToolCall[]): ChatMessages {
  return calls.map((item) => ({
    role: "tool" as const,
    tool_call_id: item.id ?? "",
    content: "ok",
    tool_failed: false,
  }));
}

function turn(...messages: ChatMessages): ChatMessages {
  return [{ role: "user", content: "go", message_id: "u1" }, ...messages];
}

function shape(items: TranscriptDisplayItem[]): string[] {
  return items.map((item) =>
    item.type === "tool_run"
      ? `run[${item.members.map((member) => member.key).join(",")}]`
      : item.key,
  );
}

function group(messages: ChatMessages, streaming = false) {
  return groupToolRuns(buildDisplayItems(messages, streaming));
}

describe("groupToolRuns segmentation", () => {
  it("groups two tool calls between prose and splits the prose message", () => {
    const first = [call("cat", { paths: "a.ts" }), call("tree")];
    const messages = turn(
      assistant("a1", "Looking around", first),
      ...results(first),
      assistant("a2", "Done"),
    );
    expect(shape(group(messages))).toEqual(["u1", "a1", "run[a1:post]", "a2"]);
  });

  it("puts reasoning of a prose message into the preceding segment", () => {
    const first = [call("cat"), call("tree")];
    const messages = turn(
      assistant("a1", "", first),
      ...results(first),
      assistant("a2", "Answer", [], { reasoning_content: "thinking" }),
    );
    expect(shape(group(messages))).toEqual(["u1", "run[a1,a2:pre]", "a2"]);
    const items = group(messages);
    const prose = items[2];
    expect(prose.type === "assistant" && prose.view).toMatchObject({
      prose: true,
      reasoning: false,
      footer: true,
    });
  });

  it("keeps a single-call segment ungrouped and unsplit", () => {
    const only = [call("cat")];
    const messages = turn(
      assistant("a1", "Reading", only),
      ...results(only),
      assistant("a2", "Done"),
    );
    const built = buildDisplayItems(messages, false);
    const items = groupToolRuns(built);
    expect(shape(items)).toEqual(["u1", "a1", "a2"]);
    expect(items[1]).toBe(built[1]);
  });

  it("never groups the tail segment", () => {
    const first = [call("cat"), call("tree")];
    const messages = turn(assistant("a1", "", first), ...results(first));
    const built = buildDisplayItems(messages, true);
    const items = groupToolRuns(built);
    expect(shape(items)).toEqual(["u1", "a1"]);
    expect(items[1]).toBe(built[1]);
  });

  it("counts calls across assistant messages and server tools", () => {
    const one = [call("cat")];
    const two = [call("tree")];
    const messages = turn(
      assistant("a1", "", one),
      ...results(one),
      assistant("a2", "", two),
      ...results(two),
      assistant("a3", "Done"),
    );
    expect(shape(group(messages))).toEqual(["u1", "run[a1,a2]", "a3"]);

    const server = turn(
      assistant("s1", "", [], { server_executed_tools: [call("web_search")] }),
      assistant("s2", "", [call("cat")]),
      assistant("s3", "Done"),
    );
    expect(shape(group(server))).toEqual(["u1", "run[s1,s2]", "s3"]);
  });

  it("does not count reasoning toward the threshold", () => {
    const one = [call("cat")];
    const messages = turn(
      assistant("a1", "", one, { reasoning_content: "hmm" }),
      ...results(one),
      assistant("a2", "Done", [], { reasoning_content: "more" }),
    );
    const built = buildDisplayItems(messages, false);
    const items = groupToolRuns(built);
    expect(shape(items)).toEqual(["u1", "a1", "a2"]);
    expect(items[2]).toBe(built[2]);
  });

  it("splits segments at static breakers and renders them standalone", () => {
    const calls = [
      call("cat"),
      call("tree"),
      call("t_ask", { questions: [] }),
      call("glob"),
      call("web"),
    ];
    const messages = turn(
      assistant("a1", "", calls),
      ...results(calls),
      assistant("a2", "Done"),
    );
    expect(shape(group(messages))).toEqual([
      "u1",
      "run[a1]",
      "a1:post:1",
      "run[a1:post:2]",
      "a2",
    ]);
    const items = group(messages);
    const breaker = items[2];
    expect(
      breaker.type === "assistant" &&
        breaker.view?.toolCalls.map((item) => item.function.name),
    ).toEqual(["t_ask"]);
  });

  it("ends a segment at non-work items such as user messages", () => {
    const first = [call("cat"), call("tree")];
    const messages = turn(assistant("a1", "", first), ...results(first), {
      role: "user",
      content: "stop",
      message_id: "u2",
    });
    expect(shape(group(messages))).toEqual(["u1", "run[a1]", "u2"]);
  });

  it("keeps identity for unchanged groups across recomputation", () => {
    const first = [call("cat"), call("tree")];
    const messages = turn(
      assistant("a1", "", first),
      ...results(first),
      assistant("a2", "Done"),
    );
    const built = buildDisplayItems(messages, false);
    const once = groupToolRuns(built);
    const twice = groupToolRuns([...built]);
    expect(twice[1]).toBe(once[1]);
  });

  it("includes diff attachments that follow assistant work", () => {
    const edit = call("mv", { source: "a", destination: "b" });
    const diff: DiffChunk = {
      file_name: "x.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: "",
      lines_add: "a\n",
    };
    const calls = [edit, call("cat")];
    const messages: ChatMessages = turn(
      assistant("a1", "", calls),
      ...results(calls),
      {
        role: "diff",
        content: [diff],
        tool_call_id: "other",
        message_id: "d1",
      },
      assistant("a2", "Done"),
    );
    const items: DisplayItem[] = buildDisplayItems(messages, false);
    expect(items.some((item) => item.type === "diff_group")).toBe(true);
    expect(shape(groupToolRuns(items))).toEqual([
      "u1",
      "run[a1,diffs-a1]",
      "a2",
    ]);
  });
});

describe("groupToolRuns review regressions", () => {
  it("keeps a citations-only assistant visible as a segment boundary", () => {
    const first = [call("cat"), call("tree")];
    const messages = turn(
      assistant("a1", "", first),
      ...results(first),
      assistant("c1", "", [], {
        citations: [
          {
            type: "web_search_result_location",
            cited_text: "x",
            url: "https://example.com",
            title: "Example",
          },
        ],
      }),
      assistant("a2", "Done"),
    );
    const built = buildDisplayItems(messages, false);
    const items = groupToolRuns(built);
    expect(shape(items)).toEqual(["u1", "run[a1]", "c1", "a2"]);
    expect(items[2]).toBe(built[2]);
  });

  it("keeps citations of a tool-bearing message out of the group", () => {
    const calls = [call("cat"), call("tree")];
    const messages = turn(
      assistant("c1", "", calls, {
        citations: [
          {
            type: "web_search_result_location",
            cited_text: "x",
            url: "https://example.com",
            title: "Example",
          },
        ],
      }),
      ...results(calls),
      assistant("a2", "Done"),
    );
    const items = group(messages);
    expect(shape(items)).toEqual(["u1", "c1", "run[c1:post]", "a2"]);
    const sources = items[1];
    expect(sources.type === "assistant" && sources.view).toMatchObject({
      sources: true,
      toolCalls: [],
    });
  });

  it("splits server-executed breakers out of groups in render order", () => {
    const server = [
      { ...call("web_search"), id: "srvtoolu_1" },
      { ...call("t_ask", { questions: [] }), id: "srvtoolu_2" },
      { ...call("web_search"), id: "srvtoolu_3" },
    ];
    const client = [call("cat")];
    const messages = turn(
      assistant("s1", "", client, { server_executed_tools: server }),
      ...results(client),
      assistant("s2", "Done"),
    );
    const items = group(messages);
    expect(shape(items)).toEqual([
      "u1",
      "s1",
      "s1:post:1",
      "run[s1:post:2]",
      "s2",
    ]);
    const names = items.map((item) =>
      item.type === "assistant"
        ? item.view?.toolCalls.map((entry) => entry.id)
        : item.type === "tool_run"
          ? item.owners.map((entry) => entry.call.id)
          : undefined,
    );
    expect(names).toEqual([
      undefined,
      ["srvtoolu_1"],
      ["srvtoolu_2"],
      ["srvtoolu_3", client[0].id],
      undefined,
    ]);
  });

  it("moves server tools of a split prose message into the group before client calls", () => {
    const server = [{ ...call("web_search"), id: "srvtoolu_9" }];
    const client = [call("cat"), call("tree")];
    const messages = turn(
      assistant("s1", "Searching", client, { server_executed_tools: server }),
      ...results(client),
      assistant("s2", "Done"),
    );
    const items = group(messages);
    const views = items.flatMap((item) =>
      item.type === "tool_run" ? item.members : [item],
    );
    for (const view of views) {
      if (view.type === "assistant") {
        expect(view.view?.serverTools ?? false).toBe(false);
      }
    }
    const run = items.find((item) => item.type === "tool_run");
    expect(
      run?.type === "tool_run" && run.owners.map((entry) => entry.call.id),
    ).toEqual(["srvtoolu_9", client[0].id, client[1].id]);
  });

  it.each([{ interrupted: true }, { _ui_only: true }])(
    "keeps %o assistant messages whole and ungrouped",
    (extra) => {
      const calls = [call("cat"), call("tree")];
      const messages = turn(
        assistant("i1", "Partial answer", calls, { extra }),
        ...results(calls),
        assistant("a2", "Done"),
      );
      const built = buildDisplayItems(messages, false);
      const items = groupToolRuns(built);
      expect(shape(items)).toEqual(["u1", "i1", "a2"]);
      expect(items[1]).toBe(built[1]);
    },
  );

  it("renders every group member as a prose-free view so footers overlay", () => {
    const one = [call("cat")];
    const two = [call("tree"), call("glob")];
    const messages = turn(
      assistant("a1", "", one),
      ...results(one),
      assistant("a2", "", two),
      ...results(two),
      assistant("a3", "Done"),
    );
    const run = group(messages).find((item) => item.type === "tool_run");
    if (run?.type !== "tool_run") throw new Error("expected a group");
    const views = run.members.map((member) =>
      member.type === "assistant" ? member.view : undefined,
    );
    expect(views.map((view) => view?.prose)).toEqual([false, false]);
    expect(views.map((view) => view?.footer)).toEqual([true, true]);
    expect(views[1]?.toolCalls).toEqual(two);
  });

  it("treats an interrupted message as a segment boundary", () => {
    const one = [call("cat"), call("tree")];
    const two = [call("glob"), call("web")];
    const messages = turn(
      assistant("a1", "", one),
      ...results(one),
      assistant("i1", "", two, { extra: { interrupted: true } }),
      ...results(two),
      assistant("a2", "Done"),
    );
    expect(shape(group(messages))).toEqual(["u1", "run[a1]", "i1", "a2"]);
  });
});

describe("summarizeToolRun", () => {
  it("orders categories, pluralizes and dedupes files", () => {
    const calls = [
      call("web", { url: "x" }),
      call("cat", { paths: "a.ts, b.ts" }),
      call("cat", { paths: "a.ts" }),
      call("shell", { command: "ls" }),
      call("search_pattern"),
      call("tree"),
      call("subagent"),
      call("tasks_set"),
      call("update_textdoc", { path: "a.ts" }),
    ];
    const summary = summarizeToolRun(calls, NO_STATES, {});
    expect(toolRunSummaryText(summary)).toBe(
      "Edited 1 file · Ran 1 command · Read 2 files · Searched 2× · Browsed 1× · Started 1 agent · Used 1 other tool",
    );
  });

  it("uses plurals for counts above one", () => {
    const calls = [
      call("shell"),
      call("process_start"),
      call("process_list"),
      call("process_kill"),
      call("t_cat", { paths: "a" }),
    ];
    expect(toolRunSummaryText(summarizeToolRun(calls, NO_STATES, {}))).toBe(
      "Ran 2 commands · Read 1 file · Used 2 other tools",
    );
  });

  it("reports unique edited files and diff stats", () => {
    const first = call("update_textdoc", { path: "a.ts" });
    const second = call("t_patch", { path: "b.ts" });
    const third = call("update_textdoc", { path: "a.ts" });
    const chunk = (file: string, add: string, remove: string): DiffChunk => ({
      file_name: file,
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_add: add,
      lines_remove: remove,
    });
    const diffs = {
      [first.id ?? ""]: [chunk("a.ts", "x\ny\n", "z\n")],
      [second.id ?? ""]: [chunk("b.ts", "q\n", "")],
      [third.id ?? ""]: [chunk("a.ts", "w\n", "")],
    };
    const summary = summarizeToolRun([first, second, third], NO_STATES, diffs);
    expect(summary.parts[0]).toEqual({
      category: "edit",
      verb: "Edited",
      rest: "2 files",
      stats: { added: 4, removed: 1 },
    });
    expect(toolRunSummaryText(summary)).toBe("Edited 2 files +4 −1");
  });

  it("omits line stats when a diff is limited", () => {
    const edit = call("update_textdoc", { path: "a.ts" });
    const huge: DiffChunk = {
      file_name: "a.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_add: "x\n".repeat(200_000),
      lines_remove: "",
    };
    const summary = summarizeToolRun([edit], NO_STATES, {
      [edit.id ?? ""]: [huge],
    });
    expect(toolRunSummaryText(summary)).toBe("Edited 1 file");
  });

  it("counts failed member calls only", () => {
    const calls = [call("shell"), call("cat")];
    const states = new Map<string, ToolRunCallState>([
      [calls[0].id ?? "", { failed: true, busy: false }],
      [calls[1].id ?? "", { failed: false, busy: false }],
      ["foreign", { failed: true, busy: false }],
    ]);
    expect(summarizeToolRun(calls, states, {}).failed).toBe(1);
  });
});

function owner(extra: Partial<AssistantMessage> = {}): AssistantMessage {
  return assistant("owner", "", [], extra);
}

function toolResult(
  id: string | undefined,
  extra: Omit<Partial<ToolResult>, "content"> = {},
): ToolResult {
  return { ...extra, tool_call_id: id ?? "", content: "" };
}

function processCompleted(exitCode: number): EventMessage {
  return {
    role: "event",
    content: "",
    message_id: "event-proc",
    subkind: "process_completed",
    source: "exec",
    payload: { status: "exited", exit_code: exitCode },
  };
}

const THREAD = "thread-1";

function state(
  item: ToolCall,
  result: ToolResult | undefined,
  folded?: EventMessage,
  message: AssistantMessage = owner(),
) {
  return toolRunCallState(item, message, result, folded, THREAD);
}

describe("toolRunCallState", () => {
  it("fails an exec call on a failed status even when tool_failed is false", () => {
    const shell = call("shell", { command: "npm test" });
    const result = toolResult(shell.id, {
      tool_failed: false,
      extra: {
        exec: { process_id: "p", status: "failed", exit_code: 1 },
      },
    });
    expect(state(shell, result)).toEqual({ failed: true, busy: false });
  });

  it("fails an exec call that exited with a non-zero code", () => {
    const shell = call("shell", { command: "npm test" });
    const result = toolResult(shell.id, {
      tool_failed: false,
      extra: {
        exec: { process_id: "p", status: "exited", exit_code: 2 },
      },
    });
    expect(state(shell, result)).toEqual({ failed: true, busy: false });
    const ok = toolResult(shell.id, {
      tool_failed: false,
      extra: {
        exec: { process_id: "p", status: "exited", exit_code: 0 },
      },
    });
    expect(state(shell, ok)).toEqual({ failed: false, busy: false });
  });

  it("uses the folded process outcome like the exec card", () => {
    const start = call("shell", { command: "dev", run_in_background: true });
    const result = toolResult(start.id, {
      extra: {
        exec: { process_id: "p", status: "running", mode: "background" },
      },
    });
    expect(state(start, result)).toEqual({ failed: false, busy: true });
    expect(state(start, result, processCompleted(2))).toEqual({
      failed: true,
      busy: false,
    });
    expect(state(start, result, processCompleted(0))).toEqual({
      failed: false,
      busy: false,
    });
  });

  it("treats every exec tool name, including aliases, through the shared model", () => {
    const wait = call("t_process_wait", { process_id: "p" });
    const result = toolResult(wait.id, { tool_failed: true });
    expect(state(wait, result).failed).toBe(true);
  });

  it("routes any tool with exec metadata through the exec model", () => {
    const runner = call("custom_runner", { target: "build" });
    const failed = toolResult(runner.id, {
      tool_failed: false,
      extra: { exec: { process_id: "p", status: "failed" } },
    });
    expect(state(runner, failed)).toEqual({ failed: true, busy: false });
    const live = toolResult(runner.id, {
      tool_failed: false,
      extra: {
        exec: { process_id: "p", status: "running", mode: "background" },
      },
    });
    expect(state(runner, live)).toEqual({ failed: false, busy: true });
  });

  it("keeps specialized renderers ahead of exec metadata", () => {
    const read = call("cat", { paths: "a.ts" });
    const live = toolResult(read.id, {
      tool_failed: false,
      extra: {
        exec: { process_id: "p", status: "running", mode: "background" },
      },
    });
    expect(state(read, live)).toEqual({ failed: false, busy: false });
  });

  it("fails server-executed tools whose provider result is an error", () => {
    const search: ToolCall = {
      ...call("web_search", { query: "x" }),
      id: "srvtoolu_1",
    };
    const message = owner({
      server_executed_tools: [search],
      server_content_blocks: [
        { type: "server_tool_use", id: "srvtoolu_1", name: "web_search" },
        {
          type: "web_search_tool_result",
          tool_use_id: "srvtoolu_1",
          content: {
            type: "web_search_tool_result_error",
            error_code: "max_uses_exceeded",
          },
        },
      ],
    });
    expect(state(search, undefined, undefined, message)).toEqual({
      failed: true,
      busy: false,
    });
    expect(
      state(
        search,
        undefined,
        undefined,
        owner({ server_executed_tools: [search] }),
      ).failed,
    ).toBe(false);
  });
});

describe("tool run duration", () => {
  it("spans min start to max completion over stamped calls", () => {
    const calls = [
      call("cat", {}, [1_000, 3_000]),
      call("tree", {}, [2_000, 9_500]),
      call("shell"),
    ];
    expect(toolRunDurationMs(calls)).toBe(8_500);
  });

  it("is omitted when no call has both stamps", () => {
    const partial: ToolCall = { ...call("cat"), started_at_ms: 5 };
    expect(toolRunDurationMs([call("cat"), partial])).toBeNull();
    expect(
      summarizeToolRun([call("cat")], NO_STATES, {}).durationMs,
    ).toBeNull();
  });

  it("formats durations", () => {
    expect(formatToolRunDuration(0)).toBe("1s");
    expect(formatToolRunDuration(400)).toBe("1s");
    expect(formatToolRunDuration(45_000)).toBe("45s");
    expect(formatToolRunDuration(134_000)).toBe("2m 14s");
    expect(formatToolRunDuration(120_000)).toBe("2m");
    expect(formatToolRunDuration(3_840_000)).toBe("1h 4m");
    expect(formatToolRunDuration(3_600_000)).toBe("1h");
  });
});
