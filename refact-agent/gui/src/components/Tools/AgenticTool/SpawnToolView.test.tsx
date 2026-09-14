import { describe, expect, test } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { SpawnToolView } from "./SpawnToolView";
import { familyModel } from "./familyFixtures";
import { MAX_DETAIL_ROWS } from "./FamilyToolCard";
import { PAGE_CHARS } from "./PagedText";

const SPAWNED = [
  "# Subagent: Fix the flaky test",
  "- Status: queued",
  "- Agent ID: bgagent-1",
  "",
  "Child trajectory: [view](refact://chat/child-1)",
].join("\n");

function renderSpawn(input: Parameters<typeof familyModel>[0]) {
  return render(
    <SpawnToolView
      model={familyModel(input)}
      content={input.content ?? null}
    />,
  );
}

describe("SpawnToolView", () => {
  test("renders a structured launch receipt", () => {
    renderSpawn({
      toolName: "spawn_agent",
      args: { task: "Fix the flaky test" },
      content: SPAWNED,
    });
    expect(screen.getByLabelText("Agent spawn receipt")).toBeInTheDocument();
    expect(screen.getByLabelText("Launch facts")).toHaveTextContent("queued");
    expect(screen.getByTestId("agentic-tool-child-note")).toHaveTextContent(
      "child-1",
    );
  });

  test("accepts the legacy one-line spawn confirmation", () => {
    renderSpawn({
      toolName: "spawn_ab",
      content: "Started 2 variants for card T-1.",
    });
    expect(screen.getByTestId("agentic-tool-summary")).toHaveTextContent(
      "Started 2 variants",
    );
  });

  test("flags an unrecognised response and still pages it", () => {
    renderSpawn({
      toolName: "spawn_agents_batch",
      content: "y".repeat(PAGE_CHARS + 5),
    });
    expect(screen.getByTestId("agentic-tool-legacy")).toBeInTheDocument();
    expect(screen.getByTestId("agentic-tool-body").textContent).toHaveLength(
      PAGE_CHARS,
    );
  });

  test("renders an explicit malformed state for unreadable arguments", () => {
    renderSpawn({
      toolName: "spawn_agent",
      argsRaw: "{not json",
      content: "Started.",
    });
    expect(screen.getByRole("alert")).toHaveTextContent("could not be read");
    expect(screen.queryByTestId("agentic-tool-summary")).toBeNull();
    expect(
      screen.getByTestId("agentic-tool-raw-args").closest("details")?.open,
    ).toBe(false);
  });

  test("keeps failure and streaming states distinct", () => {
    const view = renderSpawn({
      toolName: "spawn_agent",
      content: "Model slot is not configured",
      toolFailed: true,
    });
    expect(screen.getByTestId("agentic-tool-error")).toHaveTextContent(
      "not configured",
    );
    view.rerender(
      <SpawnToolView
        model={familyModel({
          toolName: "spawn_agent",
          hasResult: false,
          isBusy: true,
        })}
        content={null}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "No launch outcome confirmed yet",
    );
  });

  test("bounds long launch fact lists", () => {
    const lines = Array.from(
      { length: 15 },
      (_value, index) => `- Fact ${index + 1}: value ${index + 1}`,
    );
    renderSpawn({
      toolName: "spawn_agents_batch",
      content: ["# Batch", ...lines].join("\n"),
    });
    expect(
      screen.getByLabelText("Launch facts").querySelectorAll("dt"),
    ).toHaveLength(MAX_DETAIL_ROWS);
    expect(
      screen.getByText(`Showing ${MAX_DETAIL_ROWS} of 15 entries.`),
    ).toBeInTheDocument();
  });
});
