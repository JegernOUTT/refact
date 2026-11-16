import { describe, expect, it, vi } from "vitest";
import { setUpStore } from "./store";
import { applyChatEvent } from "../features/Chat/Thread/actions";
import { pythonWorkspaceApi } from "../services/refact/pythonWorkspace";
import type { PythonRunUpdate } from "../services/refact/types";
import { pythonCall } from "../__tests__/pythonFixtures";

describe("Python workspace cache invalidation", () => {
  it("throttles call_finished invalidations per chat to two seconds without throttling run completion", () => {
    const store = setUpStore();
    const original = pythonWorkspaceApi.util.invalidateTags;
    const invalidate = vi.spyOn(pythonWorkspaceApi.util, "invalidateTags");
    Object.assign(invalidate, original);
    const now = vi.spyOn(Date, "now").mockReturnValue(10000);
    let seq = 0;
    const send = (
      chat_id: string,
      update: PythonRunUpdate = { kind: "call_finished", record: pythonCall },
    ) =>
      store.dispatch(
        applyChatEvent({
          type: "python_run_update",
          chat_id,
          seq: String(++seq),
          tool_call_id: "t",
          run_id: "r",
          update,
        }),
      );
    try {
      send("throttle-a");
      expect(invalidate).toHaveBeenCalledTimes(1);
      now.mockReturnValue(11999);
      send("throttle-a");
      expect(invalidate).toHaveBeenCalledTimes(1);
      send("throttle-b");
      expect(invalidate).toHaveBeenCalledTimes(2);
      now.mockReturnValue(12000);
      send("throttle-a");
      expect(invalidate).toHaveBeenCalledTimes(3);
      send("throttle-a", { kind: "finished", status: "completed" });
      expect(invalidate).toHaveBeenCalledTimes(4);
    } finally {
      now.mockRestore();
      invalidate.mockRestore();
    }
  });
  it.each<PythonRunUpdate>([
    { kind: "started", background: false },
    { kind: "finished", status: "completed" },
  ])("invalidates for $kind", (update) => {
    const store = setUpStore();
    const original = pythonWorkspaceApi.util.invalidateTags;
    const invalidate = vi.spyOn(pythonWorkspaceApi.util, "invalidateTags");
    Object.assign(invalidate, original);
    store.dispatch(
      applyChatEvent({
        type: "python_run_update",
        chat_id: "chat-a",
        seq: "1",
        tool_call_id: "tool-a",
        run_id: "run-a",
        update,
      }),
    );
    expect(invalidate).toHaveBeenCalledWith([
      { type: "PythonWorkspace", id: "chat-a" },
    ]);
    invalidate.mockRestore();
  });
  it("does not invalidate for unrelated events", () => {
    const store = setUpStore();
    const original = pythonWorkspaceApi.util.invalidateTags;
    const invalidate = vi.spyOn(pythonWorkspaceApi.util, "invalidateTags");
    Object.assign(invalidate, original);
    store.dispatch(
      applyChatEvent({
        type: "python_run_update",
        chat_id: "chat-a",
        seq: "1",
        tool_call_id: "tool-a",
        run_id: "run-a",
        update: { kind: "stdout", text: "hello" },
      }),
    );
    expect(invalidate).not.toHaveBeenCalled();
    invalidate.mockRestore();
  });
});
