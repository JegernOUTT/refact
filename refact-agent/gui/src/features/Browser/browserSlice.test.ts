/* eslint-disable @typescript-eslint/no-non-null-assertion */
import { describe, test, expect } from "vitest";
import {
  browserSlice,
  setBrowserRuntime,
  updateBrowserStatus,
  updateBrowserFrame,
  removeBrowserRuntime,
  setPickerActive,
  toggleAttachScreenshotOnSend,
  setBrowserNotification,
  markBrowserDetached,
  markBrowserClosed,
  shiftPendingToolbarAction,
  openBrowserUi,
  closeBrowserUi,
  type BrowserState,
  type BrowserRuntime,
  type BrowserFrame,
} from "./browserSlice";
import { applyChatEvent } from "../Chat/Thread/actions";

const reducer = browserSlice.reducer;

function makeRuntime(overrides?: Partial<BrowserRuntime>): BrowserRuntime {
  return {
    runtime_id: "rt-1",
    connected: true,
    active_tab: null,
    url: null,
    title: null,
    tabs: [],
    latest_frame: null,
    picker_active: false,
    annotate_active: false,
    attach_screenshot_on_send: false,
    timeline: [],
    timeline_open: false,
    timeline_filter_source: "all",
    timeline_filter_type: null,
    notification: null,
    oversize_info: null,
    pending_toolbar_actions: [],
    ...overrides,
  };
}

function stateWith(
  chatId: string,
  runtime: BrowserRuntime,
  uiOpen = true,
): BrowserState {
  return {
    runtimes: { [chatId]: runtime },
    browserUiOpen: uiOpen ? { [chatId]: true } : {},
  };
}

describe("browserSlice", () => {
  test("initial state has empty runtimes", () => {
    const state = reducer(undefined, { type: "@@INIT" });
    expect(state.runtimes).toEqual({});
  });

  test("setBrowserRuntime adds a runtime", () => {
    const runtime = makeRuntime();
    const state = reducer(
      undefined,
      setBrowserRuntime({ chatId: "chat-1", runtime }),
    );
    expect(state.runtimes["chat-1"]).toEqual(runtime);
  });

  test("setBrowserRuntime replaces existing runtime", () => {
    const initial = stateWith("chat-1", makeRuntime({ url: "old" }));
    const newRuntime = makeRuntime({ url: "new" });
    const state = reducer(
      initial,
      setBrowserRuntime({ chatId: "chat-1", runtime: newRuntime }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.url).toBe("new");
  });

  test("updateBrowserStatus updates connected and url", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const state = reducer(
      initial,
      updateBrowserStatus({
        chatId: "chat-1",
        connected: false,
        url: "https://example.com",
        title: "Example",
      }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.connected).toBe(false);
    expect(rt!.url).toBe("https://example.com");
    expect(rt!.title).toBe("Example");
  });

  test("updateBrowserStatus does nothing for missing chatId", () => {
    const initial: BrowserState = { runtimes: {}, browserUiOpen: {} };
    const state = reducer(
      initial,
      updateBrowserStatus({ chatId: "missing", connected: true }),
    );
    expect(state.runtimes).toEqual({});
  });

  test("updateBrowserFrame sets latest frame", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const frame: BrowserFrame = {
      mime: "image/png",
      data: "base64data",
      diff_boxes: [{ x: 0, y: 0, width: 100, height: 100 }],
    };
    const state = reducer(
      initial,
      updateBrowserFrame({ chatId: "chat-1", frame }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.latest_frame).toEqual(frame);
  });

  test("removeBrowserRuntime removes a runtime", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const state = reducer(initial, removeBrowserRuntime({ chatId: "chat-1" }));
    expect(state.runtimes["chat-1"]).toBeUndefined();
  });

  test("removeBrowserRuntime preserves other runtimes", () => {
    const initial: BrowserState = {
      runtimes: {
        "chat-1": makeRuntime({ runtime_id: "rt-1" }),
        "chat-2": makeRuntime({ runtime_id: "rt-2" }),
      },
      browserUiOpen: {},
    };
    const state = reducer(initial, removeBrowserRuntime({ chatId: "chat-1" }));
    expect(state.runtimes["chat-1"]).toBeUndefined();
    const rt2 = state.runtimes["chat-2"];
    expect(rt2).toBeDefined();
    expect(rt2!.runtime_id).toBe("rt-2");
  });

  test("setPickerActive sets picker state", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const state = reducer(
      initial,
      setPickerActive({ chatId: "chat-1", active: true }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.picker_active).toBe(true);
  });

  test("toggleAttachScreenshotOnSend toggles the flag", () => {
    const initial = stateWith(
      "chat-1",
      makeRuntime({ attach_screenshot_on_send: false }),
    );
    const state1 = reducer(
      initial,
      toggleAttachScreenshotOnSend({ chatId: "chat-1" }),
    );
    const rt1 = state1.runtimes["chat-1"];
    expect(rt1).toBeDefined();
    expect(rt1!.attach_screenshot_on_send).toBe(true);

    const state2 = reducer(
      state1,
      toggleAttachScreenshotOnSend({ chatId: "chat-1" }),
    );
    const rt2 = state2.runtimes["chat-1"];
    expect(rt2).toBeDefined();
    expect(rt2!.attach_screenshot_on_send).toBe(false);
  });

  test("setBrowserNotification sets notification", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const state = reducer(
      initial,
      setBrowserNotification({
        chatId: "chat-1",
        notification: { type: "attached", message: "Browser session attached" },
      }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.notification).toEqual({
      type: "attached",
      message: "Browser session attached",
    });
  });

  test("setBrowserNotification clears notification with null", () => {
    const initial = stateWith(
      "chat-1",
      makeRuntime({
        notification: { type: "closed", message: "Closed" },
      }),
    );
    const state = reducer(
      initial,
      setBrowserNotification({ chatId: "chat-1", notification: null }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.notification).toBeNull();
  });

  test("markBrowserDetached sets disconnected and notification", () => {
    const initial = stateWith("chat-1", makeRuntime({ connected: true }));
    const state = reducer(initial, markBrowserDetached({ chatId: "chat-1" }));
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.connected).toBe(false);
    expect(rt!.notification).toEqual({
      type: "detached",
      message: "Browser session detached",
    });
  });

  test("markBrowserClosed sets disconnected with reason", () => {
    const initial = stateWith("chat-1", makeRuntime({ connected: true }));
    const state = reducer(
      initial,
      markBrowserClosed({ chatId: "chat-1", reason: "timeout" }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.connected).toBe(false);
    expect(rt!.notification).toEqual({
      type: "closed",
      message: "Browser closed: timeout",
    });
  });

  test("markBrowserDetached does nothing for missing chatId", () => {
    const initial: BrowserState = { runtimes: {}, browserUiOpen: {} };
    const state = reducer(initial, markBrowserDetached({ chatId: "missing" }));
    expect(state.runtimes).toEqual({});
  });

  test("browser_toolbar_action SSE event pushes to pending queue", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const state = reducer(
      initial,
      applyChatEvent({
        chat_id: "chat-1",
        seq: "1",
        type: "browser_toolbar_action",
        action: "screenshot",
      }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.pending_toolbar_actions).toEqual(["screenshot"]);
  });

  test("browser_toolbar_action queues multiple actions", () => {
    const initial = stateWith("chat-1", makeRuntime());
    let state = reducer(
      initial,
      applyChatEvent({
        chat_id: "chat-1",
        seq: "1",
        type: "browser_toolbar_action",
        action: "screenshot",
      }),
    );
    state = reducer(
      state,
      applyChatEvent({
        chat_id: "chat-1",
        seq: "2",
        type: "browser_toolbar_action",
        action: "summarize",
      }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.pending_toolbar_actions).toEqual(["screenshot", "summarize"]);
  });

  test("browser_toolbar_action ignores invalid action strings", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const state = reducer(
      initial,
      applyChatEvent({
        chat_id: "chat-1",
        seq: "1",
        type: "browser_toolbar_action",
        action: "invalid_action",
      }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.pending_toolbar_actions).toEqual([]);
  });

  test("browser_toolbar_action does nothing for missing runtime", () => {
    const initial: BrowserState = { runtimes: {}, browserUiOpen: {} };
    const state = reducer(
      initial,
      applyChatEvent({
        chat_id: "chat-1",
        seq: "1",
        type: "browser_toolbar_action",
        action: "screenshot",
      }),
    );
    expect(state.runtimes["chat-1"]).toBeUndefined();
  });

  test("shiftPendingToolbarAction removes first item from queue", () => {
    const initial = stateWith(
      "chat-1",
      makeRuntime({
        pending_toolbar_actions: ["screenshot", "summarize", "curl"],
      }),
    );
    const state = reducer(
      initial,
      shiftPendingToolbarAction({ chatId: "chat-1" }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.pending_toolbar_actions).toEqual(["summarize", "curl"]);
  });

  test("shiftPendingToolbarAction does nothing on empty queue", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const state = reducer(
      initial,
      shiftPendingToolbarAction({ chatId: "chat-1" }),
    );
    const rt = state.runtimes["chat-1"];
    expect(rt).toBeDefined();
    expect(rt!.pending_toolbar_actions).toEqual([]);
  });

  test("openBrowserUi sets browserUiOpen true without creating runtime", () => {
    const state = reducer(undefined, openBrowserUi({ chatId: "chat-1" }));
    expect(state.browserUiOpen["chat-1"]).toBe(true);
    expect(state.runtimes["chat-1"]).toBeUndefined();
  });

  test("closeBrowserUi sets browserUiOpen false and clears runtime", () => {
    const initial: BrowserState = {
      runtimes: { "chat-1": makeRuntime() },
      browserUiOpen: { "chat-1": true },
    };
    const state = reducer(initial, closeBrowserUi({ chatId: "chat-1" }));
    expect(state.browserUiOpen["chat-1"]).toBe(false);
    expect(state.runtimes["chat-1"]).toBeUndefined();
  });

  test("browser_closed event closes UI and clears runtime", () => {
    const initial: BrowserState = {
      runtimes: { "chat-1": makeRuntime({ runtime_id: "rt-1" }) },
      browserUiOpen: { "chat-1": true },
    };
    const state = reducer(
      initial,
      applyChatEvent({
        chat_id: "chat-1",
        seq: "1",
        type: "browser_closed",
        runtime_id: "rt-1",
        reason: "user_closed",
      }),
    );
    expect(state.browserUiOpen["chat-1"]).toBe(false);
    expect(state.runtimes["chat-1"]).toBeUndefined();
  });

  test("SSE events are ignored when browserUiOpen is false", () => {
    const initial: BrowserState = {
      runtimes: {},
      browserUiOpen: {},
    };
    const state = reducer(
      initial,
      applyChatEvent({
        chat_id: "chat-1",
        seq: "1",
        type: "browser_status",
        connected: true,
        runtime_id: "rt-1",
      }),
    );
    expect(state.runtimes["chat-1"]).toBeUndefined();
  });

  test("browser_status does not recreate runtime after UI closed", () => {
    const initial: BrowserState = {
      runtimes: {},
      browserUiOpen: { "chat-1": false },
    };
    const state = reducer(
      initial,
      applyChatEvent({
        chat_id: "chat-1",
        seq: "1",
        type: "browser_status",
        connected: true,
        runtime_id: "rt-1",
      }),
    );
    expect(state.runtimes["chat-1"]).toBeUndefined();
  });
});
