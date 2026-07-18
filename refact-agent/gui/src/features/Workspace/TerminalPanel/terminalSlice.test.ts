import { describe, expect, test } from "vitest";

import reducer, {
  activeSessionChanged,
  sessionAdded,
  sessionRemoved,
  sessionsReattached,
  sessionStatusChanged,
} from "./terminalSlice";

describe("terminalSlice", () => {
  test("stores session metadata only across tab switches", () => {
    let state = reducer(undefined, { type: "init" });
    state = reducer(
      state,
      sessionAdded({
        process_id: "one",
        title: "bash · one",
        status: "running",
      }),
    );
    state = reducer(
      state,
      sessionAdded({
        process_id: "two",
        title: "bash · two",
        status: "running",
      }),
    );
    state = reducer(state, activeSessionChanged("one"));
    state = reducer(
      state,
      sessionStatusChanged({ processId: "one", status: "exited" }),
    );

    expect(state).toEqual({
      activeProcessId: "one",
      sessions: [
        { process_id: "one", title: "bash · one", status: "exited" },
        { process_id: "two", title: "bash · two", status: "running" },
      ],
    });
    expect(JSON.stringify(state)).not.toContain("output");
  });

  test("merges reattached sessions and selects the nearest tab on close", () => {
    let state = reducer(
      undefined,
      sessionsReattached([
        { process_id: "one", title: "bash · one", status: "running" },
        { process_id: "two", title: "bash · two", status: "running" },
      ]),
    );
    state = reducer(state, activeSessionChanged("one"));
    state = reducer(state, sessionRemoved("one"));

    expect(state.activeProcessId).toBe("two");
    expect(state.sessions.map((session) => session.process_id)).toEqual([
      "two",
    ]);
  });
});
