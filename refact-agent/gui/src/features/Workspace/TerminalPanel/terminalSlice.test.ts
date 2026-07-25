import { describe, expect, test } from "vitest";

import reducer, {
  activeSessionChanged,
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
  title: `zsh · ${processId}`,
  status,
});

describe("terminalSlice", () => {
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
});
