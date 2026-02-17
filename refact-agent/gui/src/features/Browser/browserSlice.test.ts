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
  type BrowserState,
  type BrowserRuntime,
  type BrowserFrame,
} from "./browserSlice";

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
    attach_screenshot_on_send: false,
    timeline: [],
    timeline_open: false,
    timeline_filter_source: "all",
    timeline_filter_type: null,
    ...overrides,
  };
}

function stateWith(
  chatId: string,
  runtime: BrowserRuntime,
): BrowserState {
  return { runtimes: { [chatId]: runtime } };
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
    const initial: BrowserState = { runtimes: {} };
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
    const state = reducer(
      initial,
      removeBrowserRuntime({ chatId: "chat-1" }),
    );
    expect(state.runtimes["chat-1"]).toBeUndefined();
  });

  test("removeBrowserRuntime preserves other runtimes", () => {
    const initial: BrowserState = {
      runtimes: {
        "chat-1": makeRuntime({ runtime_id: "rt-1" }),
        "chat-2": makeRuntime({ runtime_id: "rt-2" }),
      },
    };
    const state = reducer(
      initial,
      removeBrowserRuntime({ chatId: "chat-1" }),
    );
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
});
