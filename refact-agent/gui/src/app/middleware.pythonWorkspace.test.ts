import { describe, expect, it, vi } from "vitest";
import { setUpStore } from "./store";
import { applyChatEvent } from "../features/Chat/Thread/actions";
import { pythonWorkspaceApi } from "../services/refact/pythonWorkspace";
import { artifactsApi } from "../services/refact/artifacts";
import { createChatWithId } from "../features/Chat/Thread/actions";
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
  it.each<PythonRunUpdate>([
    { kind: "started", background: false },
    { kind: "finished", status: "completed" },
  ])("invalidates chat artifacts for $kind", (update) => {
    const store = setUpStore();
    const original = artifactsApi.util.invalidateTags;
    const invalidate = vi.spyOn(artifactsApi.util, "invalidateTags");
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
      { type: "Artifact", id: "chat-a" },
    ]);
    invalidate.mockRestore();
  });
  const sendDiff = (
    store: ReturnType<typeof setUpStore>,
    seq: number,
    file_name: string,
  ) =>
    store.dispatch(
      applyChatEvent({
        chat_id: "chat-a",
        seq: String(seq),
        type: "message_added",
        index: seq - 1,
        message: {
          role: "diff",
          tool_call_id: `edit-${seq}`,
          content: [
            {
              file_name,
              file_action: "edit",
              line1: 1,
              line2: 1,
              lines_remove: "",
              lines_add: "<p>new</p>\n",
            },
          ],
        },
      }),
    );
  it("batches artifact invalidation for native diffs inside artifacts folders", () => {
    vi.useFakeTimers();
    const store = setUpStore();
    store.dispatch(createChatWithId({ id: "chat-a" }));
    const original = artifactsApi.util.invalidateTags;
    const invalidate = vi.spyOn(artifactsApi.util, "invalidateTags");
    Object.assign(invalidate, original);
    try {
      const root = "/proj/.refact/trajectories/chat-a/python";
      sendDiff(store, 1, `${root}/chat-a/artifacts/report.html`);
      sendDiff(store, 2, `${root}/chat-a/artifacts/charts/a.svg`);
      sendDiff(store, 3, `${root}/subchat-1/artifacts/notes.md`);
      expect(invalidate).not.toHaveBeenCalled();
      vi.advanceTimersByTime(250);
      expect(invalidate).toHaveBeenCalledTimes(1);
      expect(invalidate).toHaveBeenCalledWith([
        { type: "Artifact", id: "chat-a" },
        { type: "Artifact", id: "subchat-1" },
      ]);
    } finally {
      invalidate.mockRestore();
      vi.useRealTimers();
    }
  });
  it("does not invalidate artifacts for native diffs outside artifacts folders", () => {
    vi.useFakeTimers();
    const store = setUpStore();
    store.dispatch(createChatWithId({ id: "chat-a" }));
    const original = artifactsApi.util.invalidateTags;
    const invalidate = vi.spyOn(artifactsApi.util, "invalidateTags");
    Object.assign(invalidate, original);
    try {
      sendDiff(store, 1, "/proj/src/report.html");
      sendDiff(store, 2, "/proj/python/chat-a/report.html");
      vi.advanceTimersByTime(250);
      expect(invalidate).not.toHaveBeenCalled();
    } finally {
      invalidate.mockRestore();
      vi.useRealTimers();
    }
  });
  it("does not invalidate chat artifacts for unrelated output", () => {
    const store = setUpStore();
    const original = artifactsApi.util.invalidateTags;
    const invalidate = vi.spyOn(artifactsApi.util, "invalidateTags");
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
