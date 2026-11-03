import type { PropsWithChildren } from "react";
import type { act as domAct } from "react-dom/test-utils";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { act as testingAct, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RootState } from "../app/store";
import { useChatCommandRequest } from "./useChatCommandRequest";
import { markQueuedCommandCancelled } from "../features/Chat/Thread/actions";
import { chatReducer } from "../features/Chat/Thread/reducer";

const act = testingAct as typeof domAct;

const transport = vi.hoisted(() => {
  vi.resetModules();
  return { send: vi.fn(), cancel: vi.fn() };
});
vi.mock("../services/refact/chatCommands", async (original) => ({
  ...(await original<object>()),
  sendChatCommand: transport.send,
  cancelQueuedItem: transport.cancel,
}));

function fixture() {
  let runtime = {
    error: "original failure",
    command_acks: {} as Record<string, { accepted: boolean; result: unknown }>,
    queued_items: [] as { client_request_id: string }[],
    cancelled_command_ids: [] as string[],
    run: "old",
    value: 0,
  };
  const state = () =>
    ({
      config: { host: "ide", lspPort: 8001 },
      chat: { current_thread_id: "chat", threads: { chat: runtime } },
    }) as unknown as RootState;
  const store = configureStore({
    reducer: (_state = state(), action) => {
      const current = state();
      if (!markQueuedCommandCancelled.match(action)) return current;
      return { ...current, chat: chatReducer(current.chat, action) };
    },
  });
  const dispatch = vi.spyOn(store, "dispatch");
  const update = (patch: Partial<typeof runtime>) => {
    act(() => {
      runtime = { ...runtime, ...patch };
      store.dispatch({ type: "server" });
    });
  };
  const wrapper = ({ children }: PropsWithChildren) => (
    <Provider store={store}>{children}</Provider>
  );
  const hook = renderHook(
    () =>
      useChatCommandRequest(
        "chat",
        "stable",
        (s) => (s.chat.threads.chat as unknown as typeof runtime).run === "old",
      ),
    { wrapper },
  );
  const applied = (s: RootState) =>
    (s.chat.threads.chat as unknown as typeof runtime).value === 1;
  const start = async (type: "regenerate" | "set_params" = "regenerate") => {
    let outcome!: Promise<boolean>;
    await act(async () => {
      outcome = hook.result.current.run(
        type === "regenerate" ? { type } : { type, patch: { value: 1 } },
        applied,
        "request",
      );
      await Promise.resolve();
    });
    return { outcome };
  };
  const ack = (result: unknown = null) => ({
    request: { accepted: true, result },
  });
  return { ...hook, update, start, ack, dispatch };
}

beforeEach(() => {
  vi.useFakeTimers();
  transport.send.mockReset().mockResolvedValue("request");
  transport.cancel.mockReset().mockResolvedValue(true);
});
afterEach(() => {
  vi.useRealTimers();
});

describe("command delivery observation", () => {
  it("confirms the applied retry before classifying the new run's error", async () => {
    const f = fixture();
    const { outcome } = await f.start();
    f.update({
      run: "new",
      value: 1,
      error: "new attempt failed",
      command_acks: f.ack(),
    });
    expect(await outcome).toBe(true);
    expect(f.result.current.uncertain).toBe(false);
  });

  it("keeps an accepted queued command known beyond the acknowledgment window without resending", async () => {
    const f = fixture();
    const { outcome } = await f.start();
    f.update({
      queued_items: [{ client_request_id: "request" }],
      command_acks: f.ack(),
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(f.result.current.uncertain).toBe(false);
    expect(f.result.current.pending).toBe(true);
    expect(f.result.current).toMatchObject({
      phase: "queued",
      queuedCommandId: "request",
    });
    f.update({ queued_items: [], value: 1, run: "new" });
    expect(await outcome).toBe(true);
    expect(transport.send).toHaveBeenCalledTimes(1);
  });

  it("does not equate queued acceptance with application when the source changes", async () => {
    const f = fixture();
    const { outcome } = await f.start();
    f.update({
      run: "new",
      queued_items: [{ client_request_id: "request" }],
      command_acks: f.ack(),
    });
    expect(f.result.current.pending).toBe(true);
    f.update({ queued_items: [] });
    expect(await outcome).toBe(false);
    expect(f.result.current.uncertain).toBe(true);
  });

  it("bounds unconfirmed observation again after a known queue entry disappears", async () => {
    const f = fixture();
    const { outcome } = await f.start();
    f.update({
      queued_items: [{ client_request_id: "request" }],
      command_acks: f.ack(),
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(f.result.current.pending).toBe(true);
    f.update({ queued_items: [] });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(await outcome).toBe(false);
    expect(f.result.current).toMatchObject({
      phase: "uncertain",
      blocked: true,
    });
    expect(transport.send).toHaveBeenCalledTimes(1);
  });

  it("reconciles confirmed DELETE cancellation through the shipping Redux marker without resending", async () => {
    const f = fixture();
    const { outcome } = await f.start();
    f.update({
      queued_items: [{ client_request_id: "request" }],
      command_acks: f.ack(),
    });
    await act(async () => {
      await f.result.current.cancelQueuedRequest();
    });
    expect(transport.cancel).toHaveBeenCalledWith(
      "chat",
      "request",
      expect.anything(),
      undefined,
    );
    expect(f.dispatch).toHaveBeenCalledWith(
      markQueuedCommandCancelled({
        chatId: "chat",
        clientRequestId: "request",
      }),
    );
    expect(await outcome).toBe(false);
    expect(f.result.current).toMatchObject({
      phase: "cancelled",
      blocked: false,
      queuedCommandId: null,
    });
    expect(transport.send).toHaveBeenCalledTimes(1);
  });

  it("requires an applied acknowledgment for set_params even when the value matches", async () => {
    const f = fixture();
    const { outcome } = await f.start("set_params");
    f.update({ value: 1, command_acks: f.ack() });
    expect(f.result.current.pending).toBe(true);
    f.update({ command_acks: f.ack({ applied: true }) });
    expect(await outcome).toBe(true);
  });

  it("does not resend an uncertain request when checking status", async () => {
    const f = fixture();
    const { outcome } = await f.start();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(await outcome).toBe(false);
    act(() => {
      void f.result.current.checkStatus();
    });
    f.update({ value: 1, command_acks: f.ack() });
    expect(f.result.current.blocked).toBe(false);
    expect(transport.send).toHaveBeenCalledTimes(1);
  });
});
