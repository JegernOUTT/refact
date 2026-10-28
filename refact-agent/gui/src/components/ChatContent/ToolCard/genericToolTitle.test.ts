import { describe, expect, it } from "vitest";
import { describeToolCall, primaryArgument } from "./genericToolTitle";

describe("describeToolCall", () => {
  it.each([
    ["playwright_browser_run_code_unsafe", "{}", "Playwright", "Run code"],
    ["github_create_issue", "{}", "GitHub", "Create issue"],
    ["mcp_github_create_issue", "{}", "GitHub", "Create issue"],
    ["mcp_postgres_query", '{"sql":"select 1"}', "Postgres", "Query"],
  ])("humanizes %s", (name, args, verb, target) => {
    const title = describeToolCall(name, args);
    expect(title.verb).toBe(verb);
    expect(title.target).toBe(target);
  });

  it("labels set_trajectory_label and MCP tool search", () => {
    expect(
      describeToolCall("set_trajectory_label", '{"label":"Fix login"}'),
    ).toEqual({ verb: "Label", target: "Fix login" });
    expect(describeToolCall("mcp_tool_search", '{"query":"jira"}')).toEqual({
      verb: "MCP tools",
      target: "jira",
    });
  });

  it("unwraps mcp_call into the inner server and action", () => {
    const title = describeToolCall(
      "mcp_call",
      JSON.stringify({
        tool_name: "github_create_issue",
        args: { title: "Broken build" },
      }),
    );
    expect(title).toEqual({
      verb: "GitHub",
      target: "Create issue",
      meta: "Broken build",
    });
  });

  it("puts the primary argument in meta without key=value dumps", () => {
    const title = describeToolCall(
      "unknown_tool",
      '{"limit":3,"url":"https://example.com"}',
    );
    expect(title.verb).toBe("Unknown tool");
    expect(title.meta).toBe("https://example.com");
    expect(title.meta).not.toContain("=");
  });

  it("tolerates malformed arguments", () => {
    expect(describeToolCall("thing", "{not json")).toEqual({
      verb: "Thing",
      meta: undefined,
    });
  });
});

describe("primaryArgument", () => {
  it("falls back to the first short single-line string", () => {
    expect(primaryArgument({ body: "a\nb", owner: "acme" })).toBe("acme");
    expect(primaryArgument({ count: 2 })).toBeUndefined();
  });
});
