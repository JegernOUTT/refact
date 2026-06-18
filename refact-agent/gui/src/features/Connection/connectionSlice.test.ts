import { describe, it, expect } from "vitest";
import {
  connectionSlice,
  registerVisibleChatMount,
  unregisterVisibleChatMount,
  setConnectionSuspended,
  setSseStatus,
  selectVisibleChatMountIds,
  selectCurrentChatSseStatus,
  type ConnectionState,
} from "./connectionSlice";
import type { RootState } from "../../app/store";

function makeState(
  connection: ConnectionState,
  currentThreadId: string | null,
): RootState {
  return {
    connection,
    chat: { current_thread_id: currentThreadId },
  } as unknown as RootState;
}

describe("connectionSlice", () => {
  it("refcounts visible chat mounts", () => {
    let state = connectionSlice.reducer(
      undefined,
      registerVisibleChatMount({ chatId: "a" }),
    );
    state = connectionSlice.reducer(
      state,
      registerVisibleChatMount({ chatId: "a" }),
    );
    state = connectionSlice.reducer(
      state,
      registerVisibleChatMount({ chatId: "b" }),
    );

    expect(selectVisibleChatMountIds(makeState(state, null)).sort()).toEqual([
      "a",
      "b",
    ]);

    state = connectionSlice.reducer(
      state,
      unregisterVisibleChatMount({ chatId: "a" }),
    );
    expect(selectVisibleChatMountIds(makeState(state, null)).sort()).toEqual([
      "a",
      "b",
    ]);

    state = connectionSlice.reducer(
      state,
      unregisterVisibleChatMount({ chatId: "a" }),
    );
    expect(selectVisibleChatMountIds(makeState(state, null))).toEqual(["b"]);
  });

  it("reports connecting (not disconnected) when the current chat is mounted but has no sse row", () => {
    const state = connectionSlice.reducer(
      undefined,
      registerVisibleChatMount({ chatId: "chat-1" }),
    );

    expect(selectCurrentChatSseStatus(makeState(state, "chat-1"))).toBe(
      "connecting",
    );
  });

  it("returns null sse status while the page is suspended", () => {
    let state = connectionSlice.reducer(
      undefined,
      registerVisibleChatMount({ chatId: "chat-1" }),
    );
    state = connectionSlice.reducer(
      state,
      setSseStatus({ chatId: "chat-1", status: "disconnected" }),
    );
    expect(selectCurrentChatSseStatus(makeState(state, "chat-1"))).toBe(
      "disconnected",
    );

    state = connectionSlice.reducer(state, setConnectionSuspended(true));
    expect(selectCurrentChatSseStatus(makeState(state, "chat-1"))).toBeNull();

    state = connectionSlice.reducer(state, setConnectionSuspended(false));
    expect(selectCurrentChatSseStatus(makeState(state, "chat-1"))).toBe(
      "disconnected",
    );
  });

  it("returns null sse status when the current chat is not mounted", () => {
    const state = connectionSlice.reducer(
      undefined,
      setSseStatus({ chatId: "chat-1", status: "connected" }),
    );

    expect(selectCurrentChatSseStatus(makeState(state, "chat-1"))).toBeNull();
  });
});
