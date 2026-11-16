import { describe, expect, it } from "vitest";
import { selectPythonApprovals, selectPythonRun } from "./selectors";
import type { RootState } from "../../../app/store";
import { chatReducer } from "./reducer";
import {
  applyChatEvent,
  clearPythonLiveState,
  removePythonApproval,
  newChatAction,
} from "./actions";
import type { ChatEventEnvelope } from "../../../services/refact/chatSubscription";
import { pythonApproval, pythonCall } from "../../../__tests__/pythonFixtures";
import {
  flattenedPythonDeliveryEvent,
  flattenedPythonToolMessage,
} from "../../../__tests__/pythonWireFixtures";
import { nativeToolJobResult } from "../../../components/ChatContent/ChatContentDisplayItems";
import {
  isPythonRunExtra,
  UNREADABLE_ENGINE_WARNING,
  normalizePythonApprovalRequest,
  normalizePythonCallRecord,
  normalizePythonRunExtra,
  type ChatMessages,
} from "../../../services/refact/types";
import { restoreChat } from "./actions";
import type { ChatHistoryItem } from "../../History/historySlice";

function setup() {
  let state = chatReducer(undefined, newChatAction(undefined));
  const chat_id = state.current_thread_id;
  let seq = 0;
  return {
    send: (
      event: ChatEventEnvelope extends infer E
        ? E extends ChatEventEnvelope
          ? Omit<E, "chat_id" | "seq">
          : never
        : never,
    ) => {
      state = chatReducer(
        state,
        applyChatEvent({
          ...event,
          chat_id,
          seq: String(++seq),
        } as ChatEventEnvelope),
      );
      return state.threads[chat_id];
    },
  };
}

describe("Python runtime", () => {
  it("python run updates accumulate by call_id", () => {
    const { send } = setup();
    const base = {
      type: "python_run_update" as const,
      tool_call_id: "t",
      run_id: "r",
    };
    send({ ...base, update: { kind: "started", background: false } });
    send({ ...base, update: { kind: "call_started", record: pythonCall } });
    const finished = { ...pythonCall, status: "ok" as const, duration_ms: 10 };
    expect(
      send({ ...base, update: { kind: "call_finished", record: finished } })
        ?.pythonRuns?.t.calls,
    ).toEqual([finished]);
    expect(
      send({ ...base, update: { kind: "detached" } })?.pythonRuns?.t.status,
    ).toBe("background");
    expect(
      send({ ...base, update: { kind: "finished", status: "completed" } })
        ?.pythonRuns?.t,
    ).toMatchObject({ status: "finished", finalStatus: "completed" });
  });
  it("dedupes repeated starts and call_started and caps call upserts at 200", () => {
    const { send } = setup();
    const base = {
      type: "python_run_update" as const,
      tool_call_id: "t",
      run_id: "r",
    };
    send({ ...base, update: { kind: "started", background: false } });
    send({ ...base, update: { kind: "call_started", record: pythonCall } });
    expect(
      send({ ...base, update: { kind: "call_started", record: pythonCall } })
        ?.pythonRuns?.t.calls,
    ).toHaveLength(1);
    expect(
      send({ ...base, update: { kind: "started", background: false } })
        ?.pythonRuns?.t.calls,
    ).toHaveLength(1);
    for (let i = 0; i < 220; i++)
      send({
        ...base,
        update: {
          kind: "call_finished",
          record: { ...pythonCall, call_id: String(i) },
        },
      });
    expect(
      send({ ...base, update: { kind: "detached" } })?.pythonRuns?.t.calls,
    ).toHaveLength(200);
  });
  it("drops an active run when its owning tool call is truncated", () => {
    const { send } = setup();
    send({
      type: "message_added",
      index: 0,
      message: {
        role: "assistant",
        content: "",
        tool_calls: [
          {
            id: "t",
            index: 0,
            type: "function",
            function: { name: "python", arguments: "{}" },
          },
        ],
      },
    });
    send({
      type: "python_run_update",
      tool_call_id: "t",
      run_id: "r",
      update: { kind: "started", background: false },
    });
    expect(
      send({ type: "messages_truncated", from_index: 0 })?.pythonRuns?.t,
    ).toBeUndefined();
  });
  it("prunes finished runs on truncation but preserves active runs", () => {
    const { send } = setup();
    for (const tool_call_id of ["done", "active"])
      send({
        type: "python_run_update",
        tool_call_id,
        run_id: tool_call_id,
        update: { kind: "started", background: false },
      });
    send({
      type: "python_run_update",
      tool_call_id: "done",
      run_id: "done",
      update: { kind: "finished", status: "completed" },
    });
    expect(
      Object.keys(
        send({ type: "messages_truncated", from_index: 0 })?.pythonRuns ?? {},
      ),
    ).toEqual(["active"]);
  });
  it("prunes finished live runs only after a valid matching stored result arrives", () => {
    const { send } = setup();
    const base = {
      type: "python_run_update" as const,
      tool_call_id: "t",
      run_id: "r",
    };
    send({ ...base, update: { kind: "finished", status: "completed" } });
    const message = {
      role: "tool" as const,
      tool_call_id: "t",
      message_id: "result",
      content: "result",
    };
    expect(
      send({
        type: "message_added",
        index: 0,
        message: { ...message, extra: { python: { schema: 2 } } },
      })?.pythonRuns?.t,
    ).toBeDefined();
    const python = {
      schema: 1,
      run_id: "r",
      status: "completed",
      background: false,
      started_at_ms: 0,
      finished_at_ms: 1,
      workspace_dir: "",
      run_dir: "",
      calls: [],
      calls_total: 0,
      calls_truncated: false,
      stdout_preview: "",
      stdout_bytes: 0,
      result_preview: "",
      result_bytes: 0,
      error: null,
      files_read: [],
      files_written: [],
      artifacts: [],
      state: {
        restored: false,
        reset_reason: null,
        persisted: false,
        bytes: 0,
      },
    };
    expect(
      send({
        type: "message_updated",
        message_id: "result",
        message: {
          ...message,
          extra: { python: { ...python, run_id: "different" } },
        },
      })?.pythonRuns?.t,
    ).toBeDefined();
    expect(
      send({
        type: "message_updated",
        message_id: "result",
        message: { ...message, extra: { python } },
      })?.pythonRuns?.t,
    ).toBeUndefined();
  });
  it("clears only the requested chat live state and removes stale approvals", () => {
    let state = chatReducer(undefined, newChatAction(undefined));
    const id = state.current_thread_id;
    state = chatReducer(
      state,
      applyChatEvent({
        type: "python_approval_requested",
        chat_id: id,
        seq: "1",
        request: pythonApproval,
      }),
    );
    state = chatReducer(state, removePythonApproval({ id, requestId: "q" }));
    expect(state.threads[id]?.pythonApprovals).toEqual({});
    state = chatReducer(
      state,
      applyChatEvent({
        type: "python_run_update",
        chat_id: id,
        seq: "2",
        tool_call_id: "t",
        run_id: "r",
        update: { kind: "started", background: false },
      }),
    );
    state = chatReducer(state, clearPythonLiveState({ id }));
    expect(state.threads[id]?.pythonRuns).toEqual({});
  });
  it("approval resolved removes pending", () => {
    const { send } = setup();
    expect(
      send({ type: "python_approval_requested", request: pythonApproval })
        ?.pythonApprovals?.q,
    ).toEqual(pythonApproval);
    expect(
      send({
        type: "python_approval_resolved",
        request_id: "q",
        decision: "deny",
      })?.pythonApprovals,
    ).toEqual({});
  });
  it("stdout tail is bounded", () => {
    const { send } = setup();
    const base = {
      type: "python_run_update" as const,
      tool_call_id: "t",
      run_id: "r",
    };
    send({ ...base, update: { kind: "started", background: false } });
    send({ ...base, update: { kind: "stdout", text: "x".repeat(20000) } });
    expect(
      send({ ...base, update: { kind: "stdout", text: "end" } })?.pythonRuns?.t
        .stdout,
    ).toBe("x".repeat(16381) + "end");
  });
  it("bounds UTF-8 output without splitting characters", () => {
    const { send } = setup();
    const rt = send({
      type: "python_run_update",
      tool_call_id: "t",
      run_id: "r",
      update: { kind: "stdout", text: "猫".repeat(20000) },
    });
    expect(rt?.pythonRuns?.t.stdout).toBe("猫".repeat(5461));
  });
  it("selectors retain references and sort pending approvals", () => {
    let state = chatReducer(undefined, newChatAction(undefined));
    const chat_id = state.current_thread_id;
    state = chatReducer(
      state,
      applyChatEvent({
        type: "python_approval_requested",
        chat_id,
        seq: "1",
        request: { ...pythonApproval, created_at_ms: 2 },
      }),
    );
    state = chatReducer(
      state,
      applyChatEvent({
        type: "python_approval_requested",
        chat_id,
        seq: "2",
        request: { ...pythonApproval, request_id: "earlier", created_at_ms: 1 },
      }),
    );
    const root = { chat: state } as RootState;
    const approvals = selectPythonApprovals(root, chat_id);
    expect(approvals.map((request) => request.request_id)).toEqual([
      "earlier",
      "q",
    ]);
    expect(selectPythonApprovals(root, chat_id)).toBe(approvals);
    expect(selectPythonRun(root, chat_id, "missing")).toBeUndefined();
    expect(selectPythonApprovals(root, "missing")).toBe(
      selectPythonApprovals(root, "missing"),
    );
  });
  it("snapshot replaces python runs and approvals", () => {
    const { send } = setup();
    send({ type: "python_approval_requested", request: pythonApproval });
    const snapshot = {
      type: "snapshot" as const,
      thread: {
        id: "chat",
        title: "",
        model: "",
        mode: "agent",
        tool_use: "agent",
        boost_reasoning: false,
        context_tokens_cap: null,
        include_project_info: false,
        checkpoints_enabled: false,
        is_title_generated: false,
      },
      runtime: {
        state: "idle" as const,
        paused: false,
        error: null,
        queue_size: 0,
        pause_reasons: [],
        queued_items: [],
      },
      messages: [],
      background_agents: [],
    };
    const run = {
      tool_call_id: "t",
      run_id: "r",
      status: "background" as const,
      started_at_ms: 1,
      calls: [pythonCall],
      stdout_tail: "tail",
    };
    expect(
      send({
        ...snapshot,
        runtime: {
          ...snapshot.runtime,
          python_runs: [run],
          python_approvals: [pythonApproval],
        },
      })?.pythonRuns?.t,
    ).toMatchObject({ background: true, stdout: "tail" });
    const rt = send(snapshot);
    expect(rt?.pythonRuns).toEqual({});
    expect(rt?.pythonApprovals).toEqual({});
  });

  describe("displays", () => {
    const base = {
      type: "python_run_update" as const,
      tool_call_id: "t",
      run_id: "r",
    };
    const table = (index: number, title = `T${index}`) => ({
      index,
      kind: "table",
      title,
      columns: ["a"],
      rows: [[index]],
      truncated: false,
      path: `/run/displays/${index}.json`,
    });
    it("appends a live display by index, ignores a duplicate index and keeps order", () => {
      const { send } = setup();
      send({ ...base, update: { kind: "started", background: false } });
      send({ ...base, update: { kind: "display", display: table(2) } });
      send({ ...base, update: { kind: "display", display: table(1) } });
      const run = send({
        ...base,
        update: { kind: "display", display: table(2, "again") },
      })?.pythonRuns?.t;
      expect(run?.displays?.map((display) => display.index)).toEqual([1, 2]);
      expect(run?.displays?.[1].title).toBe("T2");
    });
    it("keeps at most 12 live displays, the lowest indexes", () => {
      const { send } = setup();
      for (let index = 13; index >= 1; index--)
        send({ ...base, update: { kind: "display", display: table(index) } });
      const run = send({ ...base, update: { kind: "detached" } })?.pythonRuns
        ?.t;
      expect(run?.displays?.map((display) => display.index)).toEqual(
        Array.from({ length: 12 }, (_, index) => index + 1),
      );
    });
    it("normalises one stored metadata object once, however often it is read", () => {
      const raw = {
        ...flattenedPythonToolMessage.python,
        displays: [table(1)],
      };
      const first = normalizePythonRunExtra(raw);
      expect(normalizePythonRunExtra(raw)).toBe(first);
      expect(isPythonRunExtra(raw)).toBe(true);
      expect(normalizePythonRunExtra({ ...raw })).not.toBe(first);
    });
    it("replaces live displays from a reconnect snapshot", () => {
      const { send } = setup();
      send({ ...base, update: { kind: "display", display: table(1) } });
      const rt = send({
        type: "snapshot",
        thread: {
          id: "chat",
          title: "",
          model: "",
          mode: "agent",
          tool_use: "agent",
          boost_reasoning: false,
          context_tokens_cap: null,
          include_project_info: false,
          checkpoints_enabled: false,
          is_title_generated: false,
        },
        runtime: {
          state: "idle",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
          python_runs: [
            {
              tool_call_id: "t",
              run_id: "r",
              status: "running",
              started_at_ms: 1,
              calls: [],
              stdout_tail: "",
              displays: [table(3), table(4)],
            },
          ],
        },
        messages: [],
        background_agents: [],
      });
      expect(
        rt?.pythonRuns?.t.displays?.map((display) => display.index),
      ).toEqual([3, 4]);
    });
    it("lifts displays and model totals from the final metadata, dropping invalid ones", () => {
      const extra = normalizePythonRunExtra({
        ...flattenedPythonToolMessage.python,
        displays: [
          table(2),
          { index: 0, kind: "table", truncated: false, path: "/x" },
          { ...table(1), rows: [[{ nested: true }, 1]] },
          "junk",
        ],
        displays_total: 3,
        displays_truncated: "no",
        llm: {
          calls: 2,
          input_tokens: 10,
          output_tokens: 5,
          models: ["m"],
        },
      });
      expect(extra?.displays?.map((display) => display.index)).toEqual([1, 2]);
      expect(extra?.displays?.[0].rows).toEqual([[null, 1]]);
      expect(extra?.displays_total).toBe(3);
      expect(extra).not.toHaveProperty("displays_truncated");
      expect(extra?.llm).toEqual({
        calls: 2,
        input_tokens: 10,
        output_tokens: 5,
        models: ["m"],
      });
      expect(
        normalizePythonRunExtra({
          ...flattenedPythonToolMessage.python,
          displays: "nope",
          llm: { calls: -1 },
        }),
      ).toBeDefined();
    });
    it("accepts model call records and the run-scoped approval", () => {
      const record = normalizePythonCallRecord({
        ...pythonCall,
        tool: "llm",
        approval: "allowed_for_run",
        model: "m",
        tokens_in: 12,
        tokens_out: "3",
      });
      expect(record).toMatchObject({
        approval: "allowed_for_run",
        model: "m",
        tokens_in: 12,
      });
      expect(record).not.toHaveProperty("tokens_out");
    });
  });

  describe("engine requests and changes", () => {
    const engine = {
      function: "settings.set",
      class: "change",
      title: "change engine settings",
      items: [{ label: "a", old: 1, new: 2 }],
    };
    it("keeps a valid engine part, reads an unknown class as risky, keeps unnamed items and drops invalid optionals", () => {
      const request = normalizePythonApprovalRequest({
        ...pythonApproval,
        tool_name: "engine.settings.set",
        engine: {
          ...engine,
          class: "catastrophic",
          scope: 3,
          warning: "",
          items: [
            { label: "a", new: { deep: [1] }, truncated: "yes" },
            { old: 1 },
          ],
        },
      });
      expect(request?.engine).toEqual({
        function: "settings.set",
        class: "risky",
        title: "change engine settings",
        items: [
          { label: "a", new: { deep: [1] } },
          { label: "(unnamed)", old: 1 },
        ],
      });
    });
    it("turns an unreadable engine part into a risky engine request without items", () => {
      const request = normalizePythonApprovalRequest({
        ...pythonApproval,
        tool_name: "engine.chats.delete",
        editing_tool: true,
        engine: { title: 3 },
      });
      expect(request?.engine).toEqual({
        function: "chats.delete",
        class: "risky",
        title: "change the engine",
        items: [],
        warning: UNREADABLE_ENGINE_WARNING,
      });
      expect(normalizePythonApprovalRequest({ engine })).toBeUndefined();
    });
    it("treats an engine tool without an engine part as a risky engine request", () => {
      const request = normalizePythonApprovalRequest({
        ...pythonApproval,
        tool_name: "engine.settings.set",
      });
      expect(request?.engine).toMatchObject({
        function: "settings.set",
        class: "risky",
        items: [],
      });
      expect(normalizePythonApprovalRequest(pythonApproval)).not.toHaveProperty(
        "engine",
      );
    });
    it("keeps entries without a usable label and counts an unreadable item list", () => {
      const request = normalizePythonApprovalRequest({
        ...pythonApproval,
        tool_name: "engine.settings.set",
        engine: {
          ...engine,
          items: [{ label: 7, new: 1 }, "raw", { label: "  ", value: 2 }],
        },
      });
      expect(request?.engine?.items).toEqual([
        { label: "(unnamed)", new: 1 },
        { label: "(unnamed)", new: "raw" },
        { label: "(unnamed)" },
      ]);
      expect(
        normalizePythonApprovalRequest({
          ...pythonApproval,
          engine: { ...engine, items: { a: 1 } },
        })?.engine?.items_dropped,
      ).toBe(1);
    });
    it("lifts applied changes and their total, skipping invalid entries", () => {
      const extra = normalizePythonRunExtra({
        ...flattenedPythonToolMessage.python,
        engine_changes: [
          { ...engine, seq: 2, at_ms: 5, scope: "project" },
          { ...engine, seq: -1, at_ms: 5 },
          "junk",
        ],
        engine_changes_total: 7,
      });
      expect(extra?.engine_changes).toEqual([
        { ...engine, seq: 2, at_ms: 5, scope: "project" },
      ]);
      expect(extra?.engine_changes_total).toBe(7);
      expect(
        normalizePythonRunExtra({
          ...flattenedPythonToolMessage.python,
          engine_changes: { nope: true },
          engine_changes_total: "7",
        }),
      ).not.toHaveProperty("engine_changes_total");
    });
    it("stores a live engine request and one from a reconnect snapshot", () => {
      const { send } = setup();
      const request = {
        ...pythonApproval,
        request_id: "e",
        tool_name: "engine.settings.set",
        engine: { ...engine, class: "destructive" as const, warning: "gone" },
      };
      expect(
        send({ type: "python_approval_requested", request })?.pythonApprovals?.e
          .engine?.class,
      ).toBe("destructive");
    });
  });

  describe("flattened engine `python` metadata", () => {
    const wire = flattenedPythonToolMessage as unknown as ChatMessages[number];
    const lifted = (messages: ChatMessages | undefined) => {
      const tool = messages?.find((message) => message.role === "tool");
      expect(tool).toBeDefined();
      expect(tool && "python" in tool).toBe(false);
      expect(isPythonRunExtra(tool?.extra?.python)).toBe(true);
      expect(tool?.extra?.privacy_shell).toBeUndefined();
    };
    it("lifts python from a live message_added event", () => {
      const { send } = setup();
      lifted(
        send({ type: "message_added", message: wire, index: 0 })?.thread
          .messages,
      );
    });
    it("lifts python from a message_updated event", () => {
      const { send } = setup();
      const { python: _python, ...plain } = flattenedPythonToolMessage;
      send({
        type: "message_added",
        message: plain as unknown as ChatMessages[number],
        index: 0,
      });
      lifted(
        send({
          type: "message_updated",
          message_id: flattenedPythonToolMessage.message_id,
          message: wire,
        })?.thread.messages,
      );
    });
    it("lifts python from a reconnect snapshot", () => {
      const { send } = setup();
      lifted(
        send({
          type: "snapshot",
          thread: {
            id: "chat",
            title: "",
            model: "",
            mode: "agent",
            tool_use: "agent",
            boost_reasoning: false,
            context_tokens_cap: null,
            include_project_info: false,
            checkpoints_enabled: false,
            is_title_generated: false,
          },
          runtime: {
            state: "idle",
            paused: false,
            error: null,
            queue_size: 0,
            pause_reasons: [],
            queued_items: [],
          },
          messages: [wire],
          background_agents: [],
        })?.thread.messages,
      );
    });
    it("lifts python from a chat restored from history", () => {
      let state = chatReducer(undefined, newChatAction(undefined));
      const item: ChatHistoryItem = {
        id: "restored",
        messages: [{ role: "user", content: "run it" }, wire],
        model: "m",
        title: "t",
        tool_use: "agent",
        mode: "agent",
        createdAt: "2026-05-31T00:00:00.000Z",
        updatedAt: "2026-05-31T00:01:00.000Z",
        boost_reasoning: false,
        include_project_info: true,
        increase_max_tokens: false,
        context_tokens_cap: undefined,
        last_user_message_id: "",
      };
      state = chatReducer(state, restoreChat(item));
      lifted(state.threads.restored?.thread.messages);
    });
    it.each(["user", "assistant"] as const)(
      "does not lift a top-level python key on a %s message",
      (role) => {
        const { send } = setup();
        const stray = {
          role,
          content: "hello",
          message_id: `stray-${role}`,
          python: flattenedPythonToolMessage.python,
        } as unknown as ChatMessages[number];
        const stored = send({ type: "message_added", message: stray, index: 0 })
          ?.thread.messages[0];
        expect(stored && "python" in stored).toBe(false);
        expect(stored?.extra?.python).toBeUndefined();
      },
    );
    it("lifts python from a background tool_job_result delivery", () => {
      const delivery =
        flattenedPythonDeliveryEvent as unknown as ChatMessages[number];
      const raw = nativeToolJobResult(delivery);
      expect(raw?.message.role).toBe("tool");
      expect(isPythonRunExtra(raw?.message.extra?.python)).toBe(true);
      const { send } = setup();
      const stored = send({
        type: "message_added",
        message: delivery,
        index: 0,
      })?.thread.messages[0];
      expect(stored && "python" in stored).toBe(false);
      const normalized = stored ? nativeToolJobResult(stored) : null;
      expect(isPythonRunExtra(normalized?.message.extra?.python)).toBe(true);
    });
  });
});
