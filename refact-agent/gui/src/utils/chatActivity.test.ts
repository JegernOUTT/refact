import { describe, expect, it } from "vitest";
import type { ChatThreadRuntime } from "../features/Chat/Thread/types";
import type { ChatMessage, ToolCall } from "../services/refact/types";
import { activityFromSessionState, getChatActivity } from "./chatActivity";
import {
  selectChatActivityById,
  selectTabsDisplayData,
} from "../features/Chat/Thread/selectors";
import type { RootState } from "../app/store";

const call = (name: string, args: unknown = {}, id = name): ToolCall => ({
  id,
  index: 0,
  function: { name, arguments: JSON.stringify(args) },
});
const assistant = (...calls: ToolCall[]): ChatMessage => ({
  role: "assistant",
  content: null,
  tool_calls: calls,
});
const result = (id: string): ChatMessage => ({
  role: "tool",
  tool_call_id: id,
  content: "done",
});
function runtime(
  messages: ChatMessage[],
  session_state = "executing_tools",
): ChatThreadRuntime {
  return {
    session_state,
    thread: { id: "chat", messages },
    confirmation: { pause: false, pause_reasons: [] },
    background_agents: { child: { status: "running" } },
  } as unknown as ChatThreadRuntime;
}
const wait = (kind: string, id = kind) =>
  call("wait", { target: { kind } }, id);

describe("chat activity", () => {
  it.each([
    ["generating", "working"],
    ["executing_tools", "running_tools"],
    ["waiting_user_input", "waiting_user"],
    ["waiting_ide", "needs_attention"],
    ["paused", "paused"],
    ["completed", "completed"],
    ["error", "error"],
    ["unknown", "idle"],
  ])("maps coarse %s truthfully", (state, expected) => {
    expect(activityFromSessionState(state)).toBe(expected);
    expect(getChatActivity(undefined, state)).toBe(expected);
  });
  it.each([
    ["agents", "waiting_agents"],
    ["task_agents", "waiting_agents"],
    ["process", "waiting_terminal"],
    ["duration", "waiting_timer"],
  ])("specializes %s waits", (kind, expected) => {
    expect(getChatActivity(runtime([assistant(wait(kind))]))).toBe(expected);
  });
  it("uses current questions and approval evidence without overriding terminal or held states", () => {
    const rt = runtime([assistant(wait("agents"))]);
    rt.thread.pending_questions = [{ status: "pending" }] as NonNullable<
      typeof rt.thread.pending_questions
    >;
    expect(getChatActivity(rt)).toBe("waiting_user");
    rt.thread.pending_questions[0].status = "answered";
    rt.confirmation.pause = true;
    rt.confirmation.pause_reasons = [
      {},
    ] as typeof rt.confirmation.pause_reasons;
    expect(getChatActivity(rt)).toBe("needs_approval");
    rt.session_state = "paused";
    expect(getChatActivity(rt)).toBe("needs_approval");
    rt.confirmation.pause_reasons = [];
    expect(getChatActivity(rt)).toBe("paused");
    rt.thread.pending_questions[0].status = "pending";
    for (const state of ["idle", "completed", "error"]) {
      rt.session_state = state;
      expect(getChatActivity(rt)).toBe(activityFromSessionState(state));
    }
    rt.session_state = "generating";
    rt.confirmation.pause = false;
    rt.thread.pending_questions = [];
    rt.error = "stale";
    expect(getChatActivity(rt)).toBe("working");
  });
  it("requires executing runtime and preserves terminal and attention precedence", () => {
    const messages = [assistant(wait("agents"))];
    for (const state of [
      "idle",
      "completed",
      "error",
      "waiting_user_input",
      "waiting_ide",
      "paused",
      "generating",
    ]) {
      expect(getChatActivity(runtime(messages, state))).toBe(
        activityFromSessionState(state),
      );
    }
    const rt = runtime(messages);
    rt.confirmation.pause = true;
    expect(getChatActivity(rt)).toBe("paused");
  });
  it("does not infer parent blocking from background agents or launched processes", () => {
    expect(getChatActivity(runtime([], "idle"))).toBe("idle");
    expect(getChatActivity(runtime([assistant(call("process_start"))]))).toBe(
      "running_tools",
    );
    expect(
      getChatActivity(
        runtime([assistant(call("shell", { run_in_background: true }))]),
      ),
    ).toBe("running_tools");
  });
  it("keeps mixed active batches generic, then recognizes unresolved siblings after partial results and hidden tails", () => {
    const messages: ChatMessage[] = [
      assistant(wait("agents", "a"), call("cat", {}, "b")),
    ];
    expect(getChatActivity(runtime(messages))).toBe("running_tools");
    messages.push(result("b"), { role: "system", content: "hidden update" });
    expect(getChatActivity(runtime(messages))).toBe("waiting_agents");
    messages.push(result("a"));
    expect(getChatActivity(runtime(messages))).toBe("running_tools");
  });
  it("matches exact ids and stops at a later assistant or user turn", () => {
    expect(
      getChatActivity(
        runtime([assistant(wait("agents", "a")), result("other")]),
      ),
    ).toBe("waiting_agents");
    expect(
      getChatActivity(runtime([assistant(wait("agents")), assistant()])),
    ).toBe("running_tools");
    expect(
      getChatActivity(
        runtime([
          assistant(wait("agents")),
          { role: "user", content: "new turn" },
        ]),
      ),
    ).toBe("running_tools");
  });
  it("ignores server calls and recognizes non-tool results", () => {
    expect(
      getChatActivity(runtime([assistant(wait("agents", "srvtoolu_1"))])),
    ).toBe("running_tools");
    const message = assistant(wait("agents", "a"));
    if (message.role === "assistant")
      message.server_executed_tools = message.tool_calls;
    expect(getChatActivity(runtime([message]))).toBe("running_tools");
    expect(
      getChatActivity(
        runtime([
          assistant(wait("agents", "a")),
          { role: "context_file", content: [], tool_call_id: "a" },
        ]),
      ),
    ).toBe("running_tools");
  });
  it("handles malformed arguments without guessing and normalizes supported names", () => {
    const broken = call("wait");
    broken.function.arguments = "{";
    expect(getChatActivity(runtime([assistant(broken)]))).toBe("running_tools");
    expect(
      getChatActivity(
        runtime([
          assistant(call("functions.wait", { target: { kind: "process" } })),
        ]),
      ),
    ).toBe("waiting_terminal");
    expect(
      getChatActivity(
        runtime([
          assistant(call("mcp__refact__wait", { target: { kind: "agents" } })),
        ]),
      ),
    ).toBe("waiting_agents");
    expect(
      getChatActivity(runtime([assistant(call("Bash", { command: "pwd" }))])),
    ).toBe("running_command");
    expect(
      getChatActivity(runtime([assistant(call("wait", { target: null }))])),
    ).toBe("running_tools");
  });
  it("keeps unknown or incomplete calls generic rather than hiding mixed execution", () => {
    for (const incomplete of [
      { index: 1, function: { name: "cat", arguments: "{}" } },
      { id: "bad", index: 1, function: null },
      null,
    ]) {
      const calls = [wait("agents"), incomplete] as unknown as ToolCall[];
      expect(getChatActivity(runtime([assistant(...calls)]))).toBe(
        "running_tools",
      );
    }
  });
  it("shares exact activity between tab and per-chat surface selectors with coarse history fallback", () => {
    const rt = runtime([assistant(wait("agents"))]);
    const state = {
      chat: { threads: { chat: rt }, open_thread_ids: ["chat", "saved"] },
      history: {
        chats: { saved: { title: "Saved", session_state: "executing_tools" } },
      },
      notifications: { pendingByThread: {}, lastSeenByThread: {} },
    } as unknown as RootState;
    const tabs = selectTabsDisplayData(state);
    expect(tabs[0].activity).toBe("waiting_agents");
    expect(selectChatActivityById(state, "chat")).toBe(tabs[0].activity);
    expect(selectChatActivityById(state, "saved")).toBe("running_tools");
  });
});
