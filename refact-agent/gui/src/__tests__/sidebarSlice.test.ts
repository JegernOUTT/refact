import { describe, expect, it } from "vitest";
import {
  resetSidebarState,
  sidebarReducer,
  sidebarSectionSnapshotReceived,
  sidebarSubscriptionStarted,
  sidebarWorkspaceChanged,
} from "../features/Sidebar/sidebarSlice";

describe("sidebarReducer", () => {
  it("tracks independent section terminal states", () => {
    let state = sidebarReducer(
      undefined,
      sidebarSubscriptionStarted({ subscriptionId: "sub", lspPort: 8001 }),
    );

    state = sidebarReducer(
      state,
      sidebarSectionSnapshotReceived({ section: "tasks", status: "ready" }),
    );
    state = sidebarReducer(
      state,
      sidebarSectionSnapshotReceived({
        section: "chats",
        status: "error",
        error: "boom",
      }),
    );

    expect(state.sections.workspace.status).toBe("loading");
    expect(state.sections.tasks.status).toBe("ready");
    expect(state.sections.chats).toEqual({ status: "error", error: "boom" });
  });

  it("resets section readiness for workspace changes without clearing subscription identity", () => {
    let state = sidebarReducer(
      undefined,
      sidebarSubscriptionStarted({ subscriptionId: "sub", lspPort: 8001 }),
    );
    state = sidebarReducer(
      state,
      sidebarSectionSnapshotReceived({ section: "tasks", status: "ready" }),
    );

    state = sidebarReducer(
      state,
      sidebarWorkspaceChanged({ subscriptionId: "sub" }),
    );

    expect(state.subscriptionId).toBe("sub");
    expect(state.lspPort).toBe(8001);
    expect(state.sections.tasks.status).toBe("loading");
  });

  it("keeps section readiness when reconnecting to the same endpoint", () => {
    let state = sidebarReducer(
      undefined,
      sidebarSubscriptionStarted({ subscriptionId: "sub", lspPort: 8001 }),
    );
    state = sidebarReducer(
      state,
      sidebarSectionSnapshotReceived({ section: "tasks", status: "ready" }),
    );

    state = sidebarReducer(
      state,
      sidebarSubscriptionStarted({ subscriptionId: "sub-2", lspPort: 8001 }),
    );

    expect(state.subscriptionId).toBe("sub-2");
    expect(state.lspPort).toBe(8001);
    expect(state.sections.tasks.status).toBe("ready");
  });

  it("clears section errors when reconnecting to the same endpoint", () => {
    let state = sidebarReducer(
      undefined,
      sidebarSubscriptionStarted({ subscriptionId: "sub", lspPort: 8001 }),
    );
    state = sidebarReducer(
      state,
      sidebarSectionSnapshotReceived({ section: "tasks", status: "ready" }),
    );
    state = sidebarReducer(
      state,
      sidebarSectionSnapshotReceived({
        section: "chats",
        status: "error",
        error: "transport error",
      }),
    );

    state = sidebarReducer(
      state,
      sidebarSubscriptionStarted({ subscriptionId: "sub-2", lspPort: 8001 }),
    );

    expect(state.sections.tasks.status).toBe("ready");
    expect(state.sections.chats).toEqual({ status: "loading", error: null });
  });

  it("resets all sections only when explicitly reset", () => {
    let state = sidebarReducer(
      undefined,
      sidebarSubscriptionStarted({ subscriptionId: "sub", lspPort: 8001 }),
    );
    state = sidebarReducer(
      state,
      sidebarSectionSnapshotReceived({ section: "tasks", status: "ready" }),
    );

    state = sidebarReducer(state, resetSidebarState({ lspPort: 9000 }));

    expect(state.subscriptionId).toBeNull();
    expect(state.lspPort).toBe(9000);
    expect(state.sections.tasks.status).toBe("loading");
  });
});
