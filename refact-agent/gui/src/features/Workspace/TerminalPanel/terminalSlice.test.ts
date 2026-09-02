import { describe, expect, test } from "vitest";

import reducer, {
  activeSessionChanged,
  clearTerminalChatState,
  expiredSessionsPruned,
  FINISHED_SESSION_TTL_MS,
  selectActiveTerminalProcessId,
  selectTerminalSessions,
  selectTerminalWorkbenchOpen,
  sessionExpiresAt,
  setTerminalWorkbenchOpen,
  sessionAdded,
  sessionRemoved,
  sessionsReattached,
  sessionStatusChanged,
  terminalSessionFromProcess,
  toggleTerminalWorkbench,
} from "./terminalSlice";

const session = (
  processId: string,
  status: "starting" | "running" | "exited" = "running",
) => ({
  process_id: processId,
  label: "zsh",
  title: `zsh · ${processId}`,
  status,
});

describe("terminalSlice", () => {
  test("finished sessions expire ten minutes after they ended", () => {
    const running = terminalSessionFromProcess({
      process_id: "run",
      status: "running",
      tty: true,
    });
    expect(sessionExpiresAt(running)).toBeNull();
    const unknownEnd = terminalSessionFromProcess({
      process_id: "old",
      status: "exited",
      tty: false,
      ended_at_ms: null,
    });
    expect(sessionExpiresAt(unknownEnd)).toBeNull();
    const finished = terminalSessionFromProcess({
      process_id: "done",
      status: "exited",
      tty: false,
      exit_code: 0,
      ended_at_ms: 1_000,
    });
    expect(sessionExpiresAt(finished)).toBe(1_000 + FINISHED_SESSION_TTL_MS);
  });

  test("prunes expired sessions except the kept one and repairs the active tab", () => {
    let state = reducer(undefined, { type: "init" });
    for (const id of ["stale", "fresh", "live", "viewed"]) {
      state = reducer(
        state,
        sessionAdded({ chatId: "chat-a", session: session(id) }),
      );
    }
    state = reducer(
      state,
      sessionStatusChanged({
        chatId: "chat-a",
        processId: "stale",
        status: "exited",
        exit_code: 1,
        ended_at_ms: 0,
      }),
    );
    state = reducer(
      state,
      sessionStatusChanged({
        chatId: "chat-a",
        processId: "fresh",
        status: "exited",
        exit_code: 0,
        ended_at_ms: FINISHED_SESSION_TTL_MS,
      }),
    );
    state = reducer(
      state,
      sessionStatusChanged({
        chatId: "chat-a",
        processId: "viewed",
        status: "killed",
        ended_at_ms: 0,
      }),
    );

    state = reducer(
      state,
      expiredSessionsPruned({
        chatId: "chat-a",
        now: FINISHED_SESSION_TTL_MS + 1,
        keepProcessId: "viewed",
      }),
    );
    expect(
      selectTerminalSessions({ terminal: state }, "chat-a").map(
        (item) => item.process_id,
      ),
    ).toEqual(["fresh", "live", "viewed"]);
    expect(selectActiveTerminalProcessId({ terminal: state }, "chat-a")).toBe(
      "viewed",
    );

    state = reducer(
      state,
      expiredSessionsPruned({
        chatId: "chat-a",
        now: FINISHED_SESSION_TTL_MS * 2 + 1,
      }),
    );
    expect(
      selectTerminalSessions({ terminal: state }, "chat-a").map(
        (item) => item.process_id,
      ),
    ).toEqual(["live"]);
    expect(selectActiveTerminalProcessId({ terminal: state }, "chat-a")).toBe(
      "live",
    );
  });

  test("keeps session metadata and active tabs isolated by chat", () => {
    let state = reducer(undefined, { type: "init" });
    state = reducer(
      state,
      sessionAdded({ chatId: "chat-a", session: session("a-one") }),
    );
    state = reducer(
      state,
      sessionAdded({ chatId: "chat-a", session: session("a-two") }),
    );
    state = reducer(
      state,
      sessionAdded({ chatId: "chat-b", session: session("b-one") }),
    );
    state = reducer(
      state,
      activeSessionChanged({ chatId: "chat-a", processId: "a-one" }),
    );
    state = reducer(
      state,
      sessionStatusChanged({
        chatId: "chat-a",
        processId: "a-one",
        status: "exited",
      }),
    );

    expect(state).toEqual({
      activeProcessIdByChat: {
        "chat-a": "a-one",
        "chat-b": "b-one",
      },
      sessionsByChat: {
        "chat-a": [{ ...session("a-one"), status: "exited" }, session("a-two")],
        "chat-b": [session("b-one")],
      },
      workbenchOpenByChat: {},
    });
    expect(selectTerminalSessions({ terminal: state }, "chat-a")).toEqual([
      { ...session("a-one"), status: "exited" },
      session("a-two"),
    ]);
    expect(selectTerminalSessions({ terminal: state }, "chat-b")).toEqual([
      session("b-one"),
    ]);
    expect(selectActiveTerminalProcessId({ terminal: state }, "chat-a")).toBe(
      "a-one",
    );
    expect(selectActiveTerminalProcessId({ terminal: state }, "chat-b")).toBe(
      "b-one",
    );
    expect(JSON.stringify(state)).not.toContain("output");
  });

  test("keeps workbench visibility collapsed by default and isolated by chat", () => {
    let state = reducer(undefined, { type: "init" });
    expect(selectTerminalWorkbenchOpen({ terminal: state }, "chat-a")).toBe(
      false,
    );

    state = reducer(state, toggleTerminalWorkbench({ chatId: "chat-a" }));
    state = reducer(
      state,
      setTerminalWorkbenchOpen({ chatId: "chat-b", open: true }),
    );

    expect(selectTerminalWorkbenchOpen({ terminal: state }, "chat-a")).toBe(
      true,
    );
    expect(selectTerminalWorkbenchOpen({ terminal: state }, "chat-b")).toBe(
      true,
    );

    state = reducer(state, toggleTerminalWorkbench({ chatId: "chat-a" }));
    expect(selectTerminalWorkbenchOpen({ terminal: state }, "chat-a")).toBe(
      false,
    );
    expect(selectTerminalWorkbenchOpen({ terminal: state }, "chat-b")).toBe(
      true,
    );
  });

  test("reattach replaces the snapshot and keeps active only while present", () => {
    let state = reducer(
      undefined,
      sessionsReattached({
        chatId: "chat-a",
        sessions: [session("stale"), session("kept")],
      }),
    );
    state = reducer(
      state,
      sessionsReattached({
        chatId: "chat-b",
        sessions: [session("other")],
      }),
    );
    state = reducer(
      state,
      activeSessionChanged({ chatId: "chat-a", processId: "kept" }),
    );
    state = reducer(
      state,
      sessionsReattached({
        chatId: "chat-a",
        sessions: [session("kept", "starting"), session("new", "exited")],
      }),
    );

    expect(state.sessionsByChat["chat-a"]).toEqual([
      session("kept", "starting"),
      session("new", "exited"),
    ]);
    expect(state.activeProcessIdByChat["chat-a"]).toBe("kept");
    expect(state.sessionsByChat["chat-b"]).toEqual([session("other")]);

    state = reducer(
      state,
      sessionsReattached({
        chatId: "chat-a",
        sessions: [session("replacement")],
      }),
    );
    expect(state.sessionsByChat["chat-a"]).toEqual([session("replacement")]);
    expect(state.activeProcessIdByChat["chat-a"]).toBe("replacement");

    state = reducer(
      state,
      sessionsReattached({ chatId: "chat-a", sessions: [] }),
    );
    expect(state.sessionsByChat["chat-a"]).toEqual([]);
    expect(state.activeProcessIdByChat["chat-a"]).toBeNull();
  });

  test("selects the nearest tab after a session closes", () => {
    let state = reducer(
      undefined,
      sessionsReattached({
        chatId: "chat-a",
        sessions: [session("one"), session("two")],
      }),
    );
    state = reducer(
      state,
      sessionRemoved({ chatId: "chat-a", processId: "one" }),
    );

    expect(state.sessionsByChat["chat-a"]).toEqual([session("two")]);
    expect(state.activeProcessIdByChat["chat-a"]).toBe("two");
  });

  test("clears every per-chat map without disturbing other chats", () => {
    let state = reducer(
      undefined,
      sessionsReattached({
        chatId: "chat-a",
        sessions: [session("a-one")],
      }),
    );
    state = reducer(
      state,
      sessionsReattached({
        chatId: "chat-b",
        sessions: [session("b-one")],
      }),
    );
    state = reducer(
      state,
      setTerminalWorkbenchOpen({ chatId: "chat-a", open: true }),
    );
    state = reducer(
      state,
      setTerminalWorkbenchOpen({ chatId: "chat-b", open: true }),
    );

    state = reducer(state, clearTerminalChatState("chat-a"));

    expect(state.sessionsByChat["chat-a"]).toBeUndefined();
    expect(state.activeProcessIdByChat["chat-a"]).toBeUndefined();
    expect(state.workbenchOpenByChat["chat-a"]).toBeUndefined();
    expect(state.sessionsByChat["chat-b"]).toEqual([session("b-one")]);
    expect(state.activeProcessIdByChat["chat-b"]).toBe("b-one");
    expect(state.workbenchOpenByChat["chat-b"]).toBe(true);
  });
});
