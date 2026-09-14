import { describe, expect, it } from "vitest";
import { settingValue, setSettingValue, settingsEqual } from "./settingPaths";

describe("nested settings", () => {
  it("updates one nested leaf without dropping unrendered fields", () => {
    const original = {
      max_queue_size: 10,
      coordination: { max_active_engine: 32, future: { keep: true } },
    };
    const updated = setSettingValue(
      original,
      "coordination.max_active_engine",
      64,
    );
    expect(updated).toEqual({
      max_queue_size: 10,
      coordination: { max_active_engine: 64, future: { keep: true } },
    });
    expect(settingValue(original, "coordination.max_active_engine")).toBe(32);
    expect(settingValue(updated, "coordination.max_active_engine")).toBe(64);
  });

  it("compares nested settings independent of key order", () => {
    expect(
      settingsEqual({ a: { x: 1, y: false } }, { a: { y: false, x: 1 } }),
    ).toBe(true);
    expect(settingsEqual({ a: { x: 1 } }, { a: { x: 2 } })).toBe(false);
    expect(settingsEqual({ a: null }, { a: {} })).toBe(false);
  });

  it("does not access inherited or malicious paths", () => {
    expect(settingValue({}, "constructor")).toBeUndefined();
    const original = { safe: 1 };
    expect(setSettingValue(original, "__proto__.bad", true)).toBe(original);
    expect(setSettingValue(original, "constructor.prototype.bad", true)).toBe(
      original,
    );
    expect(settingValue(original, "safe.child")).toBeUndefined();
  });
});
