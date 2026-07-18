import { afterEach, describe, expect, it, vi } from "vitest";

import {
  MAX_WEBUI_LOG_ENTRIES,
  clearWebuiLogEntries,
  getWebuiLogEntries,
  installWebuiConsoleCapture,
  recordWebuiLogEntry,
  subscribeWebuiLog,
} from "./webuiLog";

describe("webuiLog", () => {
  afterEach(() => {
    clearWebuiLogEntries();
  });

  it("records entries and notifies subscribers", () => {
    const listener = vi.fn();
    const unsubscribe = subscribeWebuiLog(listener);
    recordWebuiLogEntry("error", ["boom", { code: 42 }]);
    expect(listener).toHaveBeenCalledTimes(1);
    const entries = getWebuiLogEntries();
    expect(entries).toHaveLength(1);
    expect(entries[0].level).toBe("error");
    expect(entries[0].message).toContain("boom");
    expect(entries[0].message).toContain('{"code":42}');
    unsubscribe();
  });

  it("skips empty messages", () => {
    recordWebuiLogEntry("warn", ["   "]);
    expect(getWebuiLogEntries()).toHaveLength(0);
  });

  it("caps the buffer size", () => {
    for (let i = 0; i < MAX_WEBUI_LOG_ENTRIES + 10; i++) {
      recordWebuiLogEntry("warn", [`line ${i}`]);
    }
    const entries = getWebuiLogEntries();
    expect(entries).toHaveLength(MAX_WEBUI_LOG_ENTRIES);
    expect(entries[entries.length - 1].message).toBe(
      `line ${MAX_WEBUI_LOG_ENTRIES + 9}`,
    );
  });

  it("captures console levels while installed and restores on uninstall", () => {
    /* eslint-disable no-console -- exercising the console capture on purpose */
    const originalError = console.error;
    const originalWarn = console.warn;
    const originalLog = console.log;
    const originalInfo = console.info;
    const originalDebug = console.debug;
    const uninstall = installWebuiConsoleCapture();
    try {
      console.error("captured failure");
      console.warn("captured warning");
      console.log("captured log");
      console.info("captured info");
      console.debug("captured debug");
      expect(getWebuiLogEntries().map((entry) => entry.level)).toEqual([
        "error",
        "warn",
        "info",
        "info",
        "debug",
      ]);
      const messages = getWebuiLogEntries().map((entry) => entry.message);
      expect(messages).toContain("captured failure");
      expect(messages).toContain("captured warning");
      expect(messages).toContain("captured log");
      expect(messages).toContain("captured info");
      expect(messages).toContain("captured debug");
    } finally {
      uninstall();
    }
    expect(console.error).toBe(originalError);
    expect(console.warn).toBe(originalWarn);
    expect(console.log).toBe(originalLog);
    expect(console.info).toBe(originalInfo);
    expect(console.debug).toBe(originalDebug);
    console.error("not captured");
    /* eslint-enable no-console */
    const messages = getWebuiLogEntries().map((entry) => entry.message);
    expect(messages).not.toContain("not captured");
  });

  it("captures window error and unhandled rejection events", () => {
    const uninstall = installWebuiConsoleCapture();
    try {
      window.dispatchEvent(
        new ErrorEvent("error", {
          message: "boom",
          filename: "app.ts",
          lineno: 12,
        }),
      );
      const rejection = createPromiseRejectionEvent(new Error("async boom"));
      window.dispatchEvent(rejection);
    } finally {
      uninstall();
    }
    const messages = getWebuiLogEntries().map((entry) => entry.message);
    expect(messages).toContain("Uncaught: boom (app.ts:12)");
    expect(messages).toContain("Unhandled rejection: Error: async boom");
  });

  it("does not recurse when subscribers log during notification", () => {
    /* eslint-disable no-console -- exercising the console capture on purpose */
    const uninstall = installWebuiConsoleCapture();
    const unsubscribe = subscribeWebuiLog(() => {
      console.log("subscriber log");
    });
    try {
      console.error("outer error");
      expect(getWebuiLogEntries()).toHaveLength(1);
      expect(getWebuiLogEntries()[0].message).toBe("outer error");
    } finally {
      unsubscribe();
      uninstall();
    }
    /* eslint-enable no-console */
  });

  it("returns the same uninstall function for repeated installs", () => {
    const first = installWebuiConsoleCapture();
    const second = installWebuiConsoleCapture();
    expect(first).toBe(second);
    first();
  });
});

function createPromiseRejectionEvent(reason: unknown): Event {
  if (typeof PromiseRejectionEvent === "function") {
    return new PromiseRejectionEvent("unhandledrejection", {
      reason,
      promise: Promise.resolve(),
    });
  }
  return Object.assign(new Event("unhandledrejection"), { reason });
}
