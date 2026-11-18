import { describe, expect, it } from "vitest";
import type {
  AssistantMessage,
  ChatMessages,
  DiffChunk,
  EventMessage,
  ToolCall,
  ToolResult,
} from "../../services/refact";
import {
  buildDisplayItems,
  tryIncrementalDisplayItemsUpdate,
  type DisplayItem,
} from "./ChatContentDisplayItems";
import {
  formatToolRunDuration,
  groupToolRuns,
  summarizeToolRun,
  toolRunCallState,
  toolRunDurationMs,
  toolRunSummaryText,
  type DisplayItemToolRun,
  type ToolRunCallState,
  type TranscriptDisplayItem,
} from "./toolRunGroups";
import { BuddyNotices, FoldedCards } from "./ToolRunGroups.stories";

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

function notice(
  id: string,
  content: string,
  extra: Partial<EventMessage> = {},
): EventMessage {
  return {
    role: "event",
    message_id: id,
    content,
    subkind: "system_notice",
    source: "agents.message",
    ...extra,
  };
}

function diffChunk(file: string, add: string, remove = ""): DiffChunk {
  return {
    file_name: file,
    file_action: "edit",
    line1: 1,
    line2: 1,
    lines_add: add,
    lines_remove: remove,
  };
}

function runOf(items: TranscriptDisplayItem[]): DisplayItemToolRun {
  const run = items.find(
    (item): item is DisplayItemToolRun => item.type === "tool_run",
  );
  if (!run) throw new Error("expected a group");
  return run;
}

function runText(run: DisplayItemToolRun): string {
  return toolRunSummaryText(
    summarizeToolRun(
      run.owners.map((entry) => entry.call),
      NO_STATES,
      run.diffsByToolId,
      run.cards,
    ),
  );
}

const MEMORIES: ChatMessages[number] = {
  role: "context_file",
  message_id: "ctx1",
  tool_call_id: "knowledge_enrichment",
  content: [
    { file_name: "notes.md", file_content: "Prefer pnpm", line1: 1, line2: 1 },
  ],
};

const COMPRESSION: ChatMessages[number] = {
  role: "summarization",
  message_id: "sum1",
  content: "Earlier turns were summarized",
};

const SKILL: ChatMessages[number] = {
  role: "cd_instruction",
  message_id: "skill1",
  content: '💿 SKILL_ACTIVATED {"name":"review"}\nReview the diff',
};

const JOB_RESULT: EventMessage = {
  role: "event",
  content: "## Native completed output",
  subkind: "system_notice",
  source: "tool_job.completed",
  message_id: "tool_job:job:2:0",
  extra: {
    event: {
      subkind: "system_notice",
      source: "tool_job.completed",
      payload: { actor: "tool_completion", job_id: "job" },
    },
    tool_job_result: {
      actor: "tool_completion",
      job_id: "job",
      revision: 2,
      tool_name: "code_health",
      tool_call_id: "call",
      original_role: "tool",
    },
    delivery: { source: "tool_job.completed", id: "tool_job:job:2" },
  },
};

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
    const twice = groupToolRuns([...built], once);
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

describe("groupToolRuns notices and cards", () => {
  it("merges notices interleaved with tool runs into one group", () => {
    const first = [
      call("agent_message", { to: "bgagent-1", text: "Use main" }),
      call("agents_overview"),
    ];
    const waitOne = [call("wait", { target: { kind: "agents" } })];
    const waitTwo = [call("wait", { target: { kind: "agents" } })];
    const messages = turn(
      notice("e1", "[subagent question] bgagent-1 asks: which branch?"),
      notice("e2", "Lifecycle: bgagent-2 failed"),
      assistant("a1", "", first),
      ...results(first),
      notice("e3", "User approved 1 tool call (once)", {
        subkind: "tool_decision",
        source: "agents.approval",
      }),
      assistant("a2", "", waitOne),
      ...results(waitOne),
      notice("e4", "Goal pursuit nudge", { source: "chat.goal_monitor" }),
      assistant("a3", "", [], { reasoning_content: "Both agents are idle" }),
      assistant("a4", "", waitTwo),
      ...results(waitTwo),
      assistant("a5", "All agents finished."),
    );
    const items = group(messages);
    expect(shape(items)).toEqual(["u1", "run[e1,e2,a1,e3,a2,e4,a3,a4]", "a5"]);
    expect(runText(runOf(items))).toBe(
      "Sent 1 message · Checked agents 1× · Waited 2× · Received 4 notices",
    );
  });

  it("groups a notices-only segment and summarizes it as notices", () => {
    const messages = turn(
      notice("e1", "Subagent finished"),
      notice("e2", "Subagent stopped"),
      notice("e3", "Process finished"),
      assistant("a1", "Summary"),
    );
    const items = group(messages);
    expect(shape(items)).toEqual(["u1", "run[e1,e2,e3]", "a1"]);
    expect(runText(runOf(items))).toBe("Received 3 notices");
  });

  it("keeps a single notice as a plain row", () => {
    const messages = turn(
      notice("e1", "Subagent finished"),
      assistant("a1", "Done"),
    );
    const built = buildDisplayItems(messages, false);
    const items = groupToolRuns(built);
    expect(shape(items)).toEqual(["u1", "e1", "a1"]);
    expect(items[1]).toBe(built[1]);
  });

  it("counts one tool call and one notice as two steps", () => {
    const one = [call("cat", { paths: "a.ts" })];
    const messages = turn(
      assistant("a1", "", one),
      ...results(one),
      notice("e1", "Subagent finished"),
      assistant("a2", "Done"),
    );
    const items = group(messages);
    expect(shape(items)).toEqual(["u1", "run[a1,e1]", "a2"]);
    expect(runText(runOf(items))).toBe("Read 1 file · Received 1 notice");
  });

  it("folds tool job results, context files, compression and skill cards in as notices", () => {
    const one = [call("cat", { paths: "a.ts" })];
    const messages = turn(
      assistant("a1", "", one),
      ...results(one),
      MEMORIES,
      COMPRESSION,
      SKILL,
      JOB_RESULT,
      assistant("a2", "Done"),
    );
    const items = group(messages);
    expect(shape(items)).toEqual([
      "u1",
      "run[a1,ctx1,sum1,skill1,tool_job:job:2:0]",
      "a2",
    ]);
    expect(runOf(items).members.map((member) => member.type)).toEqual([
      "assistant",
      "context_files",
      "summarization",
      "skill_activated",
      "tool_job_result",
    ]);
    expect(runText(runOf(items))).toBe("Read 1 file · Received 4 notices");
  });

  it("counts a standalone diff group as edits instead of a notice", () => {
    const messages = turn(
      notice("e1", "Patch applied by a subagent"),
      {
        role: "diff",
        content: [diffChunk("x.ts", "a\nb\n", "c\n")],
        tool_call_id: "other",
        message_id: "d1",
      },
      assistant("a1", "Done"),
    );
    const items = group(messages);
    expect(shape(items)).toEqual(["u1", "run[e1,diffs-d1]", "a1"]);
    expect(runText(runOf(items))).toBe(
      "Edited 1 file +2 −1 · Received 1 notice",
    );
  });

  it.each<[string, ChatMessages[number]]>([
    ["user message", { role: "user", content: "Hold on", message_id: "b1" }],
    [
      "error card",
      { role: "error", content: "Provider failed", message_id: "b1" },
    ],
    ["plain text", { role: "plain_text", content: "Note", message_id: "b1" }],
    ["system prompt", { role: "system", content: "Rules", message_id: "b1" }],
    [
      "skill report",
      {
        role: "plain_text",
        content: "## Skill Report: review\nAll good",
        message_id: "b1",
      },
    ],
    [
      "verifier report event",
      notice("b1", "Verifier: 2 of 3 criteria met", {
        subkind: "verifier_report",
        source: "goal.verifier",
      }),
    ],
  ])("keeps a %s visible as a boundary between tool runs", (_, boundary) => {
    const one = [call("cat")];
    const two = [call("tree")];
    const messages = turn(
      assistant("a1", "", one),
      ...results(one),
      notice("e1", "First notice"),
      boundary,
      notice("e2", "Second notice"),
      assistant("a2", "", two),
      ...results(two),
      assistant("a3", "Done"),
    );
    const items = group(messages);
    expect(shape(items)).toEqual([
      "u1",
      "run[a1,e1]",
      "b1",
      "run[e2,a2]",
      "a3",
    ]);
    expect(items[2].type).not.toBe("tool_run");
  });

  it.each(["task_done", "agent_finish", "ask_questions", "t_finish", "t_ask"])(
    "renders %s standalone between notices and groups each side",
    (name) => {
      const before = [call("cat"), call("tree")];
      const breaker = [call(name, {})];
      const after = [call("glob"), call("web")];
      const messages = turn(
        assistant("a1", "", before),
        ...results(before),
        notice("e1", "Subagent finished"),
        assistant("a2", "", breaker),
        ...results(breaker),
        notice("e2", "Subagent stopped"),
        assistant("a3", "", after),
        ...results(after),
        assistant("a4", "Done"),
      );
      const items = group(messages);
      expect(shape(items)).toEqual([
        "u1",
        "run[a1,e1]",
        "a2",
        "run[e2,a3]",
        "a4",
      ]);
      const standalone = items[2];
      expect(
        standalone.type === "assistant" &&
          (standalone.view?.toolCalls ?? standalone.message.tool_calls)?.map(
            (entry) => entry.function.name,
          ),
      ).toEqual([name]);
    },
  );

  it("keeps notices after the final prose visible until the next boundary", () => {
    const messages = turn(
      assistant("a1", "Done"),
      notice("e1", "Late notice"),
      notice("e2", "Later notice"),
    );
    expect(shape(group(messages))).toEqual(["u1", "a1", "e1", "e2"]);
    const next: ChatMessages = [
      ...messages,
      { role: "user", content: "Thanks", message_id: "u2" },
    ];
    expect(shape(group(next))).toEqual(["u1", "a1", "run[e1,e2]", "u2"]);
  });

  it("keeps a live tool segment with trailing notices ungrouped", () => {
    const first = [call("cat"), call("tree")];
    const messages = turn(
      assistant("a1", "", first),
      ...results(first),
      notice("e1", "Subagent finished"),
    );
    const built = buildDisplayItems(messages, true);
    const items = groupToolRuns(built);
    expect(shape(items)).toEqual(["u1", "a1", "e1"]);
    expect(items[1]).toBe(built[1]);
  });

  it("keys a group that starts with a notice by that notice", () => {
    const one = [call("cat")];
    const messages = turn(
      notice("e1", "Subagent finished"),
      assistant("a1", "", one),
      ...results(one),
      assistant("a2", "Done"),
    );
    const built = buildDisplayItems(messages, false);
    const items = groupToolRuns(built);
    expect(shape(items)).toEqual(["u1", "run[e1,a1]", "a2"]);
    expect(items[1].key).toBe("run:e1");
    expect(groupToolRuns([...built], items)[1]).toBe(items[1]);
    const later = group([
      ...messages,
      { role: "user", content: "More", message_id: "u2" },
    ]);
    expect(later[1].key).toBe("run:e1");
  });

  it("renders the BuddyNotices story as one group followed by the prose", () => {
    const items = group(BuddyNotices.args.messages);
    expect(shape(items)).toEqual([
      "b-user",
      "run[b-n1,b-a1,b-n2,b-q1,b-thought,b-a2,b-approval,b-a3,b-goal,b-a4,b-n3,b-a5,b-a6,b-n4]",
      "b-final",
    ]);
    expect(runText(runOf(items))).toBe(
      "Ran 1 command · Ran 11 scripts · Sent 1 message · Checked agents 2× · Managed agents 2× · Waited 2× · Received 7 notices",
    );
  });

  it("renders the FoldedCards story as one group followed by the prose", () => {
    const items = group(FoldedCards.args.messages);
    expect(shape(items)).toEqual([
      "c-user",
      "run[c-a1,c-memories,c-summary,c-skill,tool_job:c-job:1:0,diffs-c-diff]",
      "c-final",
    ]);
    expect(runText(runOf(items))).toBe(
      "Edited 1 file +1 −1 · Read 1 file · Received 4 notices",
    );
  });
});

describe("groupToolRuns identity across rebuilds", () => {
  function groupsOf(items: TranscriptDisplayItem[]): DisplayItemToolRun[] {
    return items.filter(
      (item): item is DisplayItemToolRun => item.type === "tool_run",
    );
  }

  it("keeps a historical group when an appended event rebuilds the prose assistant", () => {
    const tools = [call("cat", { paths: "a.ts" }), call("tree")];
    const messages = turn(
      assistant("a1", "", tools),
      ...results(tools),
      assistant("a2", "Done", [], { reasoning_content: "Both reads agree" }),
    );
    const before = buildDisplayItems(messages, false);
    const grouped = groupToolRuns(before);
    expect(shape(grouped)).toEqual(["u1", "run[a1,a2:pre]", "a2"]);

    const next: ChatMessages = [...messages, notice("e1", "Subagent finished")];
    const after = tryIncrementalDisplayItemsUpdate(
      messages,
      next,
      before,
      false,
    );
    if (!after) throw new Error("expected an incremental update");
    const prose = (items: DisplayItem[]) =>
      items.find((item) => item.key === "a2");
    expect(prose(after)).not.toBe(prose(before));

    const regrouped = groupToolRuns(after, grouped);
    expect(shape(regrouped)).toEqual(["u1", "run[a1,a2:pre]", "a2", "e1"]);
    expect(regrouped[1]).toBe(grouped[1]);
  });

  it("keeps every historical group when a folded process event forces a full rebuild", () => {
    const dev = call("shell", {
      command: "npm run dev",
      run_in_background: true,
    });
    const read = call("cat", { paths: "a.ts" });
    const later = [call("tree"), call("glob")];
    const wireNotice: EventMessage = {
      role: "event",
      message_id: "e0",
      content: "agent bgagent-1: started",
      subkind: "system_notice",
      source: "agents.push",
      extra: {
        event: {
          subkind: "system_notice",
          source: "agents.push",
          payload: { agent_id: "bgagent-1" },
        },
      },
    };
    const messages = turn(
      wireNotice,
      assistant("a1", "", [dev, read]),
      {
        role: "tool",
        tool_call_id: dev.id ?? "",
        content: "Process started in background",
        tool_failed: false,
        extra: {
          exec: {
            process_id: "proc-dev",
            status: "running",
            mode: "background",
          },
        },
      },
      ...results([read]),
      notice("e1", "Subagent finished"),
      assistant("a2", "Started the dev server."),
      { role: "user", content: "Now look around", message_id: "u2" },
      assistant("a3", "", later),
      ...results(later),
      assistant("a4", "Looked around."),
    );
    const before = buildDisplayItems(messages, false);
    const grouped = groupToolRuns(before);
    expect(shape(grouped)).toEqual([
      "u1",
      "run[e0,a1,e1]",
      "a2",
      "u2",
      "run[a3]",
      "a4",
    ]);

    const next: ChatMessages = [
      ...messages,
      notice("e-proc", "Process finished", {
        subkind: "process_completed",
        source: "exec.registry",
        payload: { process_id: "proc-dev", status: "exited", exit_code: 0 },
      }),
    ];
    expect(
      tryIncrementalDisplayItemsUpdate(messages, next, before, false),
    ).toBeNull();
    const rebuilt = buildDisplayItems(next, false);
    const eventOf = (items: DisplayItem[]) => {
      const item = items.find((entry) => entry.key === "e0");
      return item?.type === "event" ? item.event : undefined;
    };
    expect(eventOf(rebuilt)).toBeDefined();
    expect(eventOf(rebuilt)).not.toBe(eventOf(before));

    const regrouped = groupToolRuns(rebuilt, grouped);
    expect(shape(regrouped)).toEqual(shape(grouped));
    const [first, second] = groupsOf(regrouped);
    expect(first).toBe(groupsOf(grouped)[0]);
    expect(second).toBe(groupsOf(grouped)[1]);
  });

  it("replaces a group whose member changed streaming state", () => {
    const tools = [call("cat"), call("tree")];
    const messages = turn(
      assistant("a1", "", tools),
      ...results(tools),
      assistant("a2", "Done", [], { reasoning_content: "Checking" }),
    );
    const streaming = buildDisplayItems(messages, true);
    const grouped = groupToolRuns(streaming);
    const settled = tryIncrementalDisplayItemsUpdate(
      messages,
      messages,
      streaming,
      false,
    );
    if (!settled) throw new Error("expected an incremental update");

    const run = runOf(groupToolRuns(settled, grouped));
    expect(run).not.toBe(runOf(grouped));
    expect(runOf(grouped).members.map(isStreamingMember)).toEqual([
      false,
      true,
    ]);
    expect(run.members.map(isStreamingMember)).toEqual([false, false]);
  });

  it("replaces a group whose member message changed", () => {
    const tools = [call("cat"), call("tree")];
    const first = assistant("a1", "", tools, { reasoning_content: "Look" });
    const messages = turn(first, ...results(tools), assistant("a2", "Done"));
    const before = buildDisplayItems(messages, false);
    const grouped = groupToolRuns(before);

    const updated: AssistantMessage = { ...first, reasoning_content: "Again" };
    const next = messages.map((message) =>
      message === first ? updated : message,
    );
    const after = tryIncrementalDisplayItemsUpdate(
      messages,
      next,
      before,
      false,
    );
    if (!after) throw new Error("expected an incremental update");

    const run = runOf(groupToolRuns(after, grouped));
    expect(run).not.toBe(runOf(grouped));
    expect(
      run.members[0].type === "assistant" &&
        run.members[0].message.reasoning_content,
    ).toBe("Again");
  });

  it("does not re-serve a group that vanished from an intermediate pass", () => {
    const tools = [call("cat"), call("tree")];
    const messages = turn(
      assistant("a1", "", tools),
      ...results(tools),
      assistant("a2", "Done"),
    );
    const grouped = groupToolRuns(buildDisplayItems(messages, false));
    const truncated = groupToolRuns(
      buildDisplayItems(messages.slice(0, 1), false),
      grouped,
    );
    expect(shape(truncated)).toEqual(["u1"]);

    const restored = groupToolRuns(
      buildDisplayItems(messages, false),
      truncated,
    );
    expect(shape(restored)).toEqual(shape(grouped));
    expect(runOf(restored)).not.toBe(runOf(grouped));
  });

  const NOTICES_ONLY: ChatMessages = [
    notice("e1", "Subagent finished"),
    notice("e2", "Subagent stopped"),
    { role: "plain_text", content: "Status note", message_id: "p1" },
  ];

  it("keeps a notices-only group across a full rebuild", () => {
    const grouped = groupToolRuns(buildDisplayItems(NOTICES_ONLY, false));
    expect(shape(grouped)).toEqual(["run[e1,e2]", "p1"]);

    const rebuilt = buildDisplayItems(NOTICES_ONLY, false);
    const regrouped = groupToolRuns(rebuilt, grouped);
    expect(shape(regrouped)).toEqual(["run[e1,e2]", "p1"]);
    expect(regrouped[0]).toBe(grouped[0]);
  });

  it("keeps a notices-only group when the first visible user message arrives", () => {
    const before = buildDisplayItems(NOTICES_ONLY, false);
    const grouped = groupToolRuns(before);
    const next: ChatMessages = [
      ...NOTICES_ONLY,
      { role: "user", content: "Thanks", message_id: "u1" },
    ];
    const after = tryIncrementalDisplayItemsUpdate(
      NOTICES_ONLY,
      next,
      before,
      false,
    );
    if (!after) throw new Error("expected an incremental update");

    const regrouped = groupToolRuns(after, grouped);
    expect(shape(regrouped)).toEqual(["run[e1,e2]", "p1", "u1"]);
    expect(regrouped[0]).toBe(grouped[0]);
  });

  it("does not reuse a group whose member only matches through an inherited key", () => {
    const grouped = groupToolRuns(buildDisplayItems(NOTICES_ONLY, false));
    const run = runOf(grouped);
    const [member, ...rest] = run.members;
    if (member.type !== "event") throw new Error("expected an event member");
    const stale: Record<string, unknown> = { ...member.event };
    delete stale.content;
    Object.defineProperty(stale, "__proto__", {
      value: {},
      enumerable: true,
      configurable: true,
      writable: true,
    });
    const tampered: DisplayItemToolRun = {
      ...run,
      members: [
        { ...member, event: stale as unknown as EventMessage },
        ...rest,
      ],
    };
    const previous = grouped.map((item) => (item === run ? tampered : item));

    const regrouped = groupToolRuns(
      buildDisplayItems(NOTICES_ONLY, false),
      previous,
    );
    expect(runOf(regrouped)).not.toBe(tampered);
  });

  it("never shares or displaces groups between independent transcripts", () => {
    const user: ChatMessages[number] = {
      role: "user",
      content: "go",
      message_id: "u1",
    };
    const transcript = (tools: ToolCall[]): ChatMessages => [
      user,
      assistant("a1", "", tools),
      ...results(tools),
      assistant("a2", "Done"),
    ];
    const itemsA = buildDisplayItems(
      transcript([call("cat"), call("tree")]),
      false,
    );
    const itemsB = buildDisplayItems(
      transcript([call("glob"), call("web")]),
      false,
    );
    const groupedA = groupToolRuns(itemsA);
    const groupedB = groupToolRuns(itemsB);
    expect(runOf(groupedB).key).toBe(runOf(groupedA).key);
    expect(runOf(groupedB)).not.toBe(runOf(groupedA));

    expect(runOf(groupToolRuns(itemsA, groupedA))).toBe(runOf(groupedA));
    expect(runOf(groupToolRuns(itemsB, groupedB))).toBe(runOf(groupedB));
    expect(runOf(groupToolRuns(itemsA, groupedB))).not.toBe(runOf(groupedB));
  });
});

function isStreamingMember(member: DisplayItem): boolean {
  return member.type === "assistant" && member.isStreaming;
}

describe("summarizeToolRun", () => {
  it("treats retired semantic search calls as generic historical tools", () => {
    const calls = [call("search_semantic"), call("t_search_semantic")];
    expect(toolRunSummaryText(summarizeToolRun(calls, NO_STATES, {}))).toBe(
      "Used 2 other tools",
    );
  });

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
      "Edited 1 file · Ran 1 command · Read 2 files · Searched 2× · Browsed 1× · Started 1 agent · Updated plan 1×",
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
      "Ran 2 commands · Read 1 file · Managed processes 2×",
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

  function everyCategory(count: number): ToolCall[] {
    const each = (make: (index: number) => ToolCall) =>
      Array.from({ length: count }, (_, index) => make(index));
    return [
      ...each(() => call("custom_tool")),
      ...each(() => call("create_knowledge")),
      ...each(() => call("get_goal")),
      ...each(() => call("tasks_set")),
      ...each(() => call("process_list")),
      ...each(() => call("sleep")),
      ...each(() => call("agent_cancel")),
      ...each(() => call("agent_status")),
      ...each((index) => call("agent_message", { to: `agent-${index}` })),
      ...each(() => call("subagent")),
      ...each(() => call("code_health")),
      ...each(() => call("web_search")),
      ...each(() => call("search_pattern")),
      ...each((index) => call("cat", { paths: `read-${index}.ts` })),
      ...each(() => call("python")),
      ...each(() => call("shell")),
      ...each((index) => call("update_textdoc", { path: `edit-${index}.ts` })),
    ];
  }

  it("orders every category and pluralizes counts above one", () => {
    const summary = summarizeToolRun(
      everyCategory(2),
      NO_STATES,
      {},
      {
        notices: 2,
        diffs: [],
      },
    );
    expect(toolRunSummaryText(summary)).toBe(
      "Edited 2 files · Ran 2 commands · Ran 2 scripts · Read 2 files · Searched 2× · Browsed 2× · Analyzed code 2× · Started 2 agents · Sent 2 messages · Checked agents 2× · Managed agents 2× · Waited 2× · Managed processes 2× · Updated plan 2× · Checked plan 2× · Saved 2 memories · Used 2 other tools · Received 2 notices",
    );
    expect(summary.parts.find((part) => part.category === "check")).toEqual({
      category: "check",
      verb: "Checked agents",
      rest: "2×",
    });
  });

  it("uses singular forms for single counts", () => {
    const summary = summarizeToolRun(
      everyCategory(1),
      NO_STATES,
      {},
      {
        notices: 1,
        diffs: [],
      },
    );
    expect(toolRunSummaryText(summary)).toBe(
      "Edited 1 file · Ran 1 command · Ran 1 script · Read 1 file · Searched 1× · Browsed 1× · Analyzed code 1× · Started 1 agent · Sent 1 message · Checked agents 1× · Managed agents 1× · Waited 1× · Managed processes 1× · Updated plan 1× · Checked plan 1× · Saved 1 memory · Used 1 other tool · Received 1 notice",
    );
  });

  it("summarizes a coordinator turn without falling back to other tools", () => {
    const calls = [
      call("agent_message", { to: "bgagent-1", text: "Status?" }),
      call("agents_overview"),
      call("agent_list"),
      ...Array.from({ length: 11 }, () => call("python", { code: "print(1)" })),
    ];
    expect(toolRunSummaryText(summarizeToolRun(calls, NO_STATES, {}))).toBe(
      "Ran 11 scripts · Sent 1 message · Checked agents 2×",
    );
  });

  it.each<[string, Record<string, unknown>, string]>([
    ["t_shell", {}, "Ran 1 command"],
    [
      "shell_service",
      { action: "start", command: "npm run dev" },
      "Ran 1 command",
    ],
    [
      "shell_service",
      { action: "restart", command: "npm run dev" },
      "Ran 1 command",
    ],
    [
      "shell_service",
      { action: " Start ", command: "npm run dev" },
      "Ran 1 command",
    ],
    [
      "shell_service",
      { action: "RESTART", command: "npm run dev" },
      "Ran 1 command",
    ],
    ["shell_service", { action: " Stop " }, "Managed processes 1×"],
    ["shell_service", { action: "status" }, "Managed processes 1×"],
    ["shell_service", { action: "logs" }, "Managed processes 1×"],
    ["shell_service", { action: "stop" }, "Managed processes 1×"],
    ["process_wait", {}, "Waited 1×"],
    ["t_wait", {}, "Waited 1×"],
    ["agent_wait", {}, "Waited 1×"],
    ["wait_agents", {}, "Waited 1×"],
    ["process_read", {}, "Managed processes 1×"],
    ["process_write_stdin", {}, "Managed processes 1×"],
    ["spawn_agent", {}, "Started 1 agent"],
    ["t_subagent", {}, "Started 1 agent"],
    ["agent_steer", { card_id: "T-1", text: "Rebase" }, "Sent 1 message"],
    ["task_broadcast", { text: "Stop" }, "Sent 1 message"],
    ["agent_ask_planner", {}, "Sent 1 message"],
    ["agent_peer_grant", { authority: "mail" }, "Managed agents 1×"],
    ["team_role_grant", {}, "Managed agents 1×"],
    ["work_overview", {}, "Checked agents 1×"],
    ["work_inspect", {}, "Checked agents 1×"],
    ["board_discover", {}, "Checked agents 1×"],
    ["check_agents", {}, "Checked agents 1×"],
    ["agent_lifecycle", { action: "inspect" }, "Checked agents 1×"],
    ["agent_lifecycle", { action: "  inspect " }, "Checked agents 1×"],
    ["agent_lifecycle", { action: "continue" }, "Managed agents 1×"],
    ["agent_approval", {}, "Managed agents 1×"],
    ["restart_agent", {}, "Managed agents 1×"],
    ["t_set_tasks", {}, "Updated plan 1×"],
    ["update_plan", {}, "Updated plan 1×"],
    ["validate_goal", {}, "Updated plan 1×"],
    ["pause_goal", {}, "Updated plan 1×"],
    ["get_goal", {}, "Checked plan 1×"],
    ["t_get_goal", {}, "Checked plan 1×"],
    ["get_plan", {}, "Checked plan 1×"],
    ["t_save_knowledge", {}, "Saved 1 memory"],
    ["task_mem_save", {}, "Saved 1 memory"],
    ["codegraph_overview", {}, "Analyzed code 1×"],
    ["design_system", {}, "Analyzed code 1×"],
    ["contrast_audit", {}, "Browsed 1×"],
    ["chrome_navigate", {}, "Browsed 1×"],
    ["schedule", {}, "Used 1 other tool"],
    ["pause", {}, "Used 1 other tool"],
    ["progress_report", {}, "Used 1 other tool"],
    ["board_get", {}, "Used 1 other tool"],
  ])("classifies %s %j as %s", (name, args, expected) => {
    expect(
      toolRunSummaryText(summarizeToolRun([call(name, args)], NO_STATES, {})),
    ).toBe(expected);
  });

  it("counts sent messages per call instead of inferring recipients", () => {
    const calls = [
      call("agent_message", { to: "bgagent-1" }),
      call("agent_message", { to: "bgagent-1" }),
      call("agent_steer", { card_id: "T-1", text: "Rebase first" }),
      call("task_broadcast", { text: "Stop" }),
      call("agent_peer_grant", { recipient_root: "chat-2" }),
    ];
    expect(toolRunSummaryText(summarizeToolRun(calls, NO_STATES, {}))).toBe(
      "Sent 4 messages · Managed agents 1×",
    );
  });

  it("counts standalone diff cards toward edited files and line stats", () => {
    const edit = call("update_textdoc", { path: "a.ts" });
    const withCall = summarizeToolRun(
      [edit],
      NO_STATES,
      { [edit.id ?? ""]: [diffChunk("a.ts", "x\ny\n", "z\n")] },
      {
        notices: 0,
        diffs: [diffChunk("b.ts", "q\n"), diffChunk("a.ts", "w\n")],
      },
    );
    expect(toolRunSummaryText(withCall)).toBe("Edited 2 files +4 −1");
    const cardsOnly = summarizeToolRun(
      [],
      NO_STATES,
      {},
      {
        notices: 1,
        diffs: [diffChunk("b.ts", "q\n")],
      },
    );
    expect(toolRunSummaryText(cardsOnly)).toBe(
      "Edited 1 file +1 −0 · Received 1 notice",
    );
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
