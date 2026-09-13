import React from "react";
import { Provider } from "react-redux";
import { renderHook } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setUpStore } from "../../../app/store";
import { updateConfig } from "../../Config/configSlice";
import {
  executeBuddyAction,
  type BuddyActionOutcome,
} from "../executeBuddyAction";
import { useBuddyControlAction } from "./useBuddyControlAction";

vi.hoisted(() => vi.resetModules());
vi.mock("../executeBuddyAction", () => ({ executeBuddyAction: vi.fn() }));
vi.mock("./useExecuteBuddyAction", () => ({
  BUDDY_ACTION_SUCCESS: { status: "success" },
  formatOpportunityActionError: (error: unknown) => String(error),
  useExecuteBuddyAction: () => vi.fn(),
}));

const control = {
  id: "care",
  label: "Play",
  action: "care_play",
  style: "primary" as const,
};

function deferred() {
  let resolve!: (value: BuddyActionOutcome) => void;
  const promise = new Promise<BuddyActionOutcome>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function setup() {
  const store = setUpStore({
    config: { lspPort: 8001, host: "web", themeProps: {} },
  });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  const hook = renderHook(({ owner }) => useBuddyControlAction(owner), {
    wrapper,
    initialProps: { owner: "first" },
  });
  return { ...hook, store };
}

describe("useBuddyControlAction", () => {
  beforeEach(() => {
    vi.mocked(executeBuddyAction).mockReset();
  });

  it("returns success and releases pending state after completion", async () => {
    vi.mocked(executeBuddyAction).mockResolvedValue({ status: "ok" });
    const { result } = setup();
    await act(async () => {
      expect(await result.current.control(control)).toEqual({
        status: "success",
      });
    });
    expect(result.current.pendingActionIds.size).toBe(0);
  });

  it("suppresses duplicate effects while retaining the original request", async () => {
    const request = deferred();
    vi.mocked(executeBuddyAction).mockReturnValue(request.promise);
    const { result } = setup();
    let first!: ReturnType<typeof result.current.control>;
    act(() => {
      first = result.current.control(control);
    });
    await act(async () => {
      expect(await result.current.control(control)).toEqual({
        status: "ignored",
        reason: "duplicate",
      });
    });
    expect(executeBuddyAction).toHaveBeenCalledTimes(1);
    expect(result.current.pendingActionIds.size).toBe(1);
    await act(async () => {
      request.resolve({ status: "ok" });
      expect(await first).toEqual({ status: "success" });
    });
    expect(result.current.pendingActionIds.size).toBe(0);
  });

  it("captures rejection without reporting success", async () => {
    vi.mocked(executeBuddyAction).mockRejectedValue(new Error("unavailable"));
    const { result } = setup();
    await act(async () => {
      expect(await result.current.control(control)).toMatchObject({
        status: "failed",
        message: "Error: unavailable",
      });
    });
    expect(result.current.pendingActionIds.size).toBe(0);
  });

  it("invalidates post-await effects on unmount", async () => {
    const request = deferred();
    vi.mocked(executeBuddyAction).mockReturnValue(request.promise);
    const { result, unmount } = setup();
    let first!: ReturnType<typeof result.current.control>;
    act(() => {
      first = result.current.control(control);
    });
    const isCurrent = vi.mocked(executeBuddyAction).mock.calls[0][3];
    expect(isCurrent?.()).toBe(true);
    unmount();
    expect(isCurrent?.()).toBe(false);
    request.resolve({ status: "ok" });
    expect(await first).toEqual({ status: "ignored", reason: "stale" });
  });

  it("does not let an old owner completion release the new owner's same-key request", async () => {
    const old = deferred();
    const current = deferred();
    vi.mocked(executeBuddyAction)
      .mockReturnValueOnce(old.promise)
      .mockReturnValueOnce(current.promise);
    const { result, rerender } = setup();
    let first!: ReturnType<typeof result.current.control>;
    let second!: ReturnType<typeof result.current.control>;
    act(() => {
      first = result.current.control(control);
    });
    rerender({ owner: "second" });
    act(() => {
      second = result.current.control(control);
    });
    await act(async () => {
      old.resolve({ status: "ok" });
      expect(await first).toEqual({ status: "ignored", reason: "stale" });
      expect(await result.current.control(control)).toEqual({
        status: "ignored",
        reason: "duplicate",
      });
    });
    expect(result.current.pendingActionIds.size).toBe(1);
    await act(async () => {
      current.resolve({ status: "ok" });
      expect(await second).toEqual({ status: "success" });
    });
    expect(result.current.pendingActionIds.size).toBe(0);
  });

  it("invalidates a result when backend URL changes without a display-name change", async () => {
    const request = deferred();
    vi.mocked(executeBuddyAction).mockReturnValue(request.promise);
    const { result, store } = setup();
    let first!: ReturnType<typeof result.current.control>;
    act(() => {
      first = result.current.control(control);
    });
    const isCurrent = vi.mocked(executeBuddyAction).mock.calls[0][3];
    act(() => {
      store.dispatch(updateConfig({ lspUrl: "http://127.0.0.1:8002" }));
    });
    expect(isCurrent?.()).toBe(false);
    await act(async () => {
      request.resolve({ status: "ok" });
      expect(await first).toEqual({ status: "ignored", reason: "stale" });
    });
    expect(result.current.pendingActionIds.size).toBe(0);
  });
});
