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

  it("captures console.error and console.warn while installed", () => {
    /* eslint-disable no-console -- exercising the console capture on purpose */
    const uninstall = installWebuiConsoleCapture();
    try {
      console.error("captured failure");
      console.warn("captured warning");
      const messages = getWebuiLogEntries().map((entry) => entry.message);
      expect(messages).toContain("captured failure");
      expect(messages).toContain("captured warning");
    } finally {
      uninstall();
    }
    console.error("not captured");
    /* eslint-enable no-console */
    const messages = getWebuiLogEntries().map((entry) => entry.message);
    expect(messages).not.toContain("not captured");
  });

  it("returns the same uninstall function for repeated installs", () => {
    const first = installWebuiConsoleCapture();
    const second = installWebuiConsoleCapture();
    expect(first).toBe(second);
    first();
  });
});
