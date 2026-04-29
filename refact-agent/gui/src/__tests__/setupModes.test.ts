import { describe, test, expect } from "vitest";
import {
  SETUP_MODES,
  SETUP_MODE_IDS,
  isValidSetupMode,
} from "../features/Setup/setupModes";

describe("SETUP_MODES", () => {
  test("contains the generic setup mode", () => {
    expect(SETUP_MODES.some((m) => m.mode === "setup")).toBe(true);
  });

  test("contains all five specific setup modes", () => {
    const modes = SETUP_MODES.map((m) => m.mode);
    expect(modes).toContain("setup_skills");
    expect(modes).toContain("setup_agents_md");
    expect(modes).toContain("setup_mcp");
    expect(modes).toContain("setup_commands");
    expect(modes).toContain("setup_subagents");
  });

  test("every entry has a non-empty label and mode", () => {
    for (const m of SETUP_MODES) {
      expect(m.label.length).toBeGreaterThan(0);
      expect(m.mode.length).toBeGreaterThan(0);
    }
  });

  test("SETUP_MODE_IDS mirrors SETUP_MODES", () => {
    for (const m of SETUP_MODES) {
      expect(SETUP_MODE_IDS.has(m.mode)).toBe(true);
    }
    expect(SETUP_MODE_IDS.size).toBe(SETUP_MODES.length);
  });
});

describe("isValidSetupMode", () => {
  test("returns true for all known modes", () => {
    for (const m of SETUP_MODES) {
      expect(isValidSetupMode(m.mode)).toBe(true);
    }
  });

  test("returns false for empty string", () => {
    expect(isValidSetupMode("")).toBe(false);
  });

  test("returns false for arbitrary unknown mode", () => {
    expect(isValidSetupMode("agent")).toBe(false);
    expect(isValidSetupMode("task_planner")).toBe(false);
    expect(isValidSetupMode("unknown_mode")).toBe(false);
    expect(isValidSetupMode("configurator")).toBe(false);
  });

  test("returns false for a partial prefix match", () => {
    expect(isValidSetupMode("setup_")).toBe(false);
    expect(isValidSetupMode("setup_skill")).toBe(false);
  });

  test("open_setup_mode action falls back to 'setup' for invalid param", () => {
    const resolve = (param: string | undefined) => {
      const raw = param ?? "";
      return isValidSetupMode(raw) ? raw : "setup";
    };

    expect(resolve(undefined)).toBe("setup");
    expect(resolve("")).toBe("setup");
    expect(resolve("agent")).toBe("setup");
    expect(resolve("setup_skills")).toBe("setup_skills");
    expect(resolve("setup_mcp")).toBe("setup_mcp");
    expect(resolve("setup")).toBe("setup");
  });
});
