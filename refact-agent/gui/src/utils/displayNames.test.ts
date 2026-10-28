import { describe, expect, it } from "vitest";
import {
  expandTruncatedAgentTitle,
  humanizeIdentifier,
  stripAgentKindPrefix,
} from "./displayNames";

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

  it("splits leading and embedded acronyms", () => {
    expect(humanizeIdentifier("HTTPRequest")).toBe("Http request");
    expect(humanizeIdentifier("MCPError")).toBe("Mcp error");
    expect(humanizeIdentifier("parseHTTPResponse")).toBe("Parse http response");
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

describe("stripAgentKindPrefix", () => {
  it("strips repeated agent kind prefixes case-insensitively", () => {
    expect(stripAgentKindPrefix("Subagent: Subagent: Implement X")).toBe(
      "Implement X",
    );
    expect(stripAgentKindPrefix("  delegate : SUBAGENT:Fix Y")).toBe("Fix Y");
  });

  it("strips markdown emphasis around the title", () => {
    expect(stripAgentKindPrefix("**Subagent: Build `gui`**")).toBe("Build gui");
  });

  it("keeps titles that only mention the kind later", () => {
    expect(stripAgentKindPrefix("Review subagent: output")).toBe(
      "Review subagent: output",
    );
  });
});

describe("expandTruncatedAgentTitle", () => {
  const task =
    "Implement approved Buddy HOME overhaul in active worktree and verify it";

  it("prefers the full task when the title is a truncated prefix", () => {
    expect(
      expandTruncatedAgentTitle(
        "Subagent: Implement approved Buddy HOME overhaul in active w…",
        task,
      ),
    ).toBe(task);
  });

  it("keeps titles that are not truncated or do not match the task", () => {
    expect(expandTruncatedAgentTitle("Subagent: Short", task)).toBe(
      "Subagent: Short",
    );
    expect(expandTruncatedAgentTitle("Subagent: Other work…", task)).toBe(
      "Subagent: Other work…",
    );
    expect(expandTruncatedAgentTitle("Subagent: Implement…", undefined)).toBe(
      "Subagent: Implement…",
    );
  });
});
