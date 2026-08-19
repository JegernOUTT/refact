import { describe, expect, it } from "vitest";
import { humanizeIdentifier } from "./displayNames";

describe("humanizeIdentifier", () => {
  it("uses curated labels for known identifiers", () => {
    expect(humanizeIdentifier("task_planner")).toBe("Task Planner");
    expect(humanizeIdentifier("needs_work")).toBe("Needs work");
    expect(humanizeIdentifier("goal_pursuit")).toBe("Goal pursuit");
    expect(humanizeIdentifier("ProviderTransient")).toBe(
      "Temporary provider issue",
    );
    expect(humanizeIdentifier("budget_exhausted")).toBe("Budget exhausted");
  });

  it("sentence-cases unknown snake_case identifiers", () => {
    expect(humanizeIdentifier("some_new_kind")).toBe("Some new kind");
  });

  it("sentence-cases unknown PascalCase identifiers", () => {
    expect(humanizeIdentifier("SomeNewCategory")).toBe("Some new category");
  });

  it("sentence-cases kebab-case and camelCase", () => {
    expect(humanizeIdentifier("mode-switch")).toBe("Mode switch");
    expect(humanizeIdentifier("modeSwitch")).toBe("Mode switch");
  });

  it("never returns raw identifier formats for multi-word input", () => {
    for (const raw of ["foo_bar", "FooBar", "foo-bar"]) {
      const out = humanizeIdentifier(raw);
      expect(out).not.toMatch(/[_-]/);
      expect(out).not.toMatch(/[a-z][A-Z]/);
    }
  });

  it("passes through empty and single-word input safely", () => {
    expect(humanizeIdentifier("")).toBe("");
    expect(humanizeIdentifier("agent")).toBe("Agent");
    expect(humanizeIdentifier("frobnicate")).toBe("Frobnicate");
  });
});
