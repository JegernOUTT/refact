import { describe, expect, test } from "vitest";

import reducer, {
  activeSessionChanged,
  clearTerminalChatState,
  selectActiveTerminalProcessId,
  selectTerminalSessions,
  selectTerminalWorkbenchOpen,
  setTerminalWorkbenchOpen,
  sessionAdded,
  sessionRemoved,
  sessionsReattached,
  sessionStatusChanged,
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
  test.each(["exited", "failed", "killed", "timed_out"] as const)(
    "closes %s sessions synchronously and repairs selection only in their chat",
    (status) => {
      let state = reducer(undefined, { type: "init" });
      for (const id of ["first", "second"]) {
        state = reducer(
          state,
          sessionAdded({ chatId: "chat-a", session: session(id) }),
        );
      }
      state = reducer(
        state,
        sessionAdded({ chatId: "chat-b", session: session("first") }),
      );
      state = reducer(
        state,
        activeSessionChanged({ chatId: "chat-a", processId: "first" }),
      );
      state = reducer(
        state,
        sessionStatusChanged({ chatId: "chat-a", processId: "first", status }),
      );

      expect(selectTerminalSessions({ terminal: state }, "chat-a")).toEqual([
        session("second"),
      ]);
      expect(selectActiveTerminalProcessId({ terminal: state }, "chat-a")).toBe(
        "second",
      );
      expect(selectTerminalSessions({ terminal: state }, "chat-b")).toEqual([
        session("first"),
      ]);

      state = reducer(
        state,
        sessionStatusChanged({ chatId: "chat-a", processId: "second", status }),
      );
      expect(selectTerminalSessions({ terminal: state }, "chat-a")).toEqual([]);
      expect(
        selectActiveTerminalProcessId({ terminal: state }, "chat-a"),
      ).toBeNull();
    },
  );

  test("ignores a delayed spawn response and stale list for a completed process", () => {
    let state = reducer(
      undefined,
      sessionStatusChanged({
        chatId: "chat-a",
        processId: "quick",
        status: "exited",
      }),
    );
    state = reducer(
      state,
      sessionAdded({ chatId: "chat-a", session: session("quick") }),
    );
    state = reducer(
      state,
      sessionsReattached({ chatId: "chat-a", sessions: [session("quick")] }),
    );
    expect(selectTerminalSessions({ terminal: state }, "chat-a")).toEqual([]);
    expect(
      selectActiveTerminalProcessId({ terminal: state }, "chat-a"),
    ).toBeNull();
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

  test("reattach merges live processes and does not reopen finished sessions", () => {
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
      session("stale"),
    ]);
    expect(state.activeProcessIdByChat["chat-a"]).toBe("kept");
    expect(state.sessionsByChat["chat-b"]).toEqual([session("other")]);

    state = reducer(
      state,
      sessionsReattached({
        chatId: "chat-a",
        sessions: [session("kept", "exited"), session("replacement")],
      }),
    );
    expect(state.sessionsByChat["chat-a"]).toEqual([
      session("replacement"),
      session("stale"),
    ]);
    expect(state.activeProcessIdByChat["chat-a"]).toBe("stale");

    state = reducer(
      state,
      sessionsReattached({
        chatId: "chat-a",
        sessions: [
          session("replacement", "exited"),
          session("stale", "exited"),
        ],
      }),
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
