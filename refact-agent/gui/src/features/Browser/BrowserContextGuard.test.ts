import { describe, it, expect } from "vitest";
import { estimateSize, formatKB } from "./BrowserContextGuard.utils";
import type { BrowserContextOversizeInfo } from "./browserSlice";
import {
  browserSlice,
  setBrowserRuntime,
  setBrowserContextOversize,
  clearBrowserContextOversize,
} from "./browserSlice";
import type { BrowserRuntime } from "./browserSlice";

const makeRuntime = (overrides?: Partial<BrowserRuntime>): BrowserRuntime => ({
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
});

const sampleInfo: BrowserContextOversizeInfo = {
  pending_message_id: "msg-123",
  total_bytes: 150_000,
  action_count: 200,
  action_bytes: 60_000,
  console_count: 100,
  console_bytes: 40_000,
  network_count: 50,
  network_bytes: 30_000,
  mutation_bytes: 20_000,
};

describe("formatKB", () => {
  it("formats bytes < 1024 as B", () => {
    expect(formatKB(500)).toBe("500 B");
  });

  it("formats bytes >= 1024 as KB", () => {
    expect(formatKB(2048)).toBe("2 KB");
  });

  it("rounds to nearest KB", () => {
    expect(formatKB(1536)).toBe("2 KB");
  });

  it("handles zero", () => {
    expect(formatKB(0)).toBe("0 B");
  });

  it("handles large values", () => {
    expect(formatKB(1_048_576)).toBe("1024 KB");
  });
});

describe("estimateSize", () => {
  it("returns total when all sections included at full count", () => {
    const result = estimateSize(sampleInfo, {
      includeActions: true,
      includeConsole: true,
      includeNetwork: true,
      includeMutations: true,
      includeScreenshot: false,
      lastNActions: 200,
      lastNConsole: 100,
      lastNNetwork: 50,
    });
    expect(result).toBe(150_000);
  });

  it("returns zero when nothing is included", () => {
    const result = estimateSize(sampleInfo, {
      includeActions: false,
      includeConsole: false,
      includeNetwork: false,
      includeMutations: false,
      includeScreenshot: false,
      lastNActions: 200,
      lastNConsole: 100,
      lastNNetwork: 50,
    });
    expect(result).toBe(0);
  });

  it("scales actions proportionally with slider", () => {
    const result = estimateSize(sampleInfo, {
      includeActions: true,
      includeConsole: false,
      includeNetwork: false,
      includeMutations: false,
      includeScreenshot: false,
      lastNActions: 100,
      lastNConsole: 0,
      lastNNetwork: 0,
    });
    expect(result).toBe(30_000);
  });

  it("caps lastN to action_count", () => {
    const result = estimateSize(sampleInfo, {
      includeActions: true,
      includeConsole: false,
      includeNetwork: false,
      includeMutations: false,
      includeScreenshot: false,
      lastNActions: 500,
      lastNConsole: 0,
      lastNNetwork: 0,
    });
    expect(result).toBe(60_000);
  });

  it("returns only mutations when only mutations included", () => {
    const result = estimateSize(sampleInfo, {
      includeActions: false,
      includeConsole: false,
      includeNetwork: false,
      includeMutations: true,
      includeScreenshot: false,
      lastNActions: 0,
      lastNConsole: 0,
      lastNNetwork: 0,
    });
    expect(result).toBe(20_000);
  });

  it("handles zero counts gracefully", () => {
    const emptyInfo: BrowserContextOversizeInfo = {
      pending_message_id: "msg-empty",
      total_bytes: 0,
      action_count: 0,
      action_bytes: 0,
      console_count: 0,
      console_bytes: 0,
      network_count: 0,
      network_bytes: 0,
      mutation_bytes: 0,
    };
    const result = estimateSize(emptyInfo, {
      includeActions: true,
      includeConsole: true,
      includeNetwork: true,
      includeMutations: true,
      includeScreenshot: true,
      lastNActions: 10,
      lastNConsole: 10,
      lastNNetwork: 10,
    });
    expect(result).toBe(0);
  });

  it("scales console proportionally", () => {
    const result = estimateSize(sampleInfo, {
      includeActions: false,
      includeConsole: true,
      includeNetwork: false,
      includeMutations: false,
      includeScreenshot: false,
      lastNActions: 0,
      lastNConsole: 50,
      lastNNetwork: 0,
    });
    expect(result).toBe(20_000);
  });

  it("scales network proportionally", () => {
    const result = estimateSize(sampleInfo, {
      includeActions: false,
      includeConsole: false,
      includeNetwork: true,
      includeMutations: false,
      includeScreenshot: false,
      lastNActions: 0,
      lastNConsole: 0,
      lastNNetwork: 25,
    });
    expect(result).toBe(15_000);
  });

  it("combines partial sections correctly", () => {
    const result = estimateSize(sampleInfo, {
      includeActions: true,
      includeConsole: true,
      includeNetwork: false,
      includeMutations: true,
      includeScreenshot: false,
      lastNActions: 100,
      lastNConsole: 50,
      lastNNetwork: 0,
    });
    expect(result).toBe(30_000 + 20_000 + 20_000);
  });
});

describe("browserSlice oversize reducers", () => {
  it("setBrowserContextOversize sets info", () => {
    let state = browserSlice.reducer(undefined, { type: "init" });
    state = browserSlice.reducer(
      state,
      setBrowserRuntime({ chatId: "chat-1", runtime: makeRuntime() }),
    );
    state = browserSlice.reducer(
      state,
      setBrowserContextOversize({ chatId: "chat-1", info: sampleInfo }),
    );
    expect(state.runtimes["chat-1"]?.oversize_info).toEqual(sampleInfo);
  });

  it("clearBrowserContextOversize clears info", () => {
    let state = browserSlice.reducer(undefined, { type: "init" });
    state = browserSlice.reducer(
      state,
      setBrowserRuntime({
        chatId: "chat-1",
        runtime: makeRuntime({ oversize_info: sampleInfo }),
      }),
    );
    state = browserSlice.reducer(
      state,
      clearBrowserContextOversize({ chatId: "chat-1" }),
    );
    expect(state.runtimes["chat-1"]?.oversize_info).toBeNull();
  });

  it("setBrowserContextOversize ignores missing runtime", () => {
    const state = browserSlice.reducer(undefined, { type: "init" });
    const nextState = browserSlice.reducer(
      state,
      setBrowserContextOversize({ chatId: "missing", info: sampleInfo }),
    );
    expect(nextState.runtimes.missing).toBeUndefined();
  });

  it("preserves pending_message_id in oversize info", () => {
    let state = browserSlice.reducer(undefined, { type: "init" });
    state = browserSlice.reducer(
      state,
      setBrowserRuntime({ chatId: "chat-1", runtime: makeRuntime() }),
    );
    state = browserSlice.reducer(
      state,
      setBrowserContextOversize({ chatId: "chat-1", info: sampleInfo }),
    );
    expect(state.runtimes["chat-1"]?.oversize_info?.pending_message_id).toBe(
      "msg-123",
    );
  });
});

describe("browser_context_decision command shape", () => {
  it("decision payload includes all required fields", () => {
    const decision = {
      type: "browser_context_decision" as const,
      pending_message_id: "msg-123",
      include_actions: true,
      include_console: true,
      include_network: false,
      include_mutations: true,
      include_screenshot: false,
      last_n_actions: 50,
      last_n_console: 100,
      last_n_network: null,
    };
    expect(decision.type).toBe("browser_context_decision");
    expect(decision.pending_message_id).toBe("msg-123");
    expect(decision.include_actions).toBe(true);
    expect(decision.include_network).toBe(false);
    expect(decision.last_n_actions).toBe(50);
    expect(decision.last_n_network).toBeNull();
  });

  it("skip context decision has all includes false", () => {
    const decision = {
      type: "browser_context_decision" as const,
      pending_message_id: "msg-456",
      include_actions: false,
      include_console: false,
      include_network: false,
      include_mutations: false,
      include_screenshot: false,
      last_n_actions: 0,
      last_n_console: 0,
      last_n_network: 0,
    };
    expect(decision.include_actions).toBe(false);
    expect(decision.include_console).toBe(false);
    expect(decision.include_network).toBe(false);
    expect(decision.include_mutations).toBe(false);
    expect(decision.include_screenshot).toBe(false);
  });
});
