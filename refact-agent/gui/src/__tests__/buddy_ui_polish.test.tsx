import { describe, expect, it, vi } from "vitest";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { setUpStore } from "../app/store";
import { actionLabel } from "../features/Buddy/buddyOpportunityActions";
import { BuddyHome } from "../features/Buddy/BuddyHome";
import { formatCompactNumber } from "../features/Buddy/buddyUtils";
import { setBuddySnapshot } from "../features/Buddy/buddySlice";
import { navigateFromBuddyPage } from "../features/Buddy/executeBuddyAction";
import { push } from "../features/Pages/pagesSlice";
import type { BuddyPage } from "../features/Buddy/types";
import {
  CONFIG_STATE,
  makePolishSnapshot as makeSnapshot,
} from "../features/Buddy/__tests__/buddyHomeFixtures";

type CapturedThunk = (
  dispatch: (action: unknown) => unknown,
  getState: () => unknown,
  extra: unknown,
) => unknown;

type TestDispatch = (action: unknown) => unknown;

function readGuiSource(path: string): Promise<string> {
  return readFile(resolve(process.cwd(), "src", path), "utf8");
}

function isSetupModeCreateAction(action: unknown): boolean {
  if (typeof action !== "object" || action === null) return false;
  const candidate = action as { payload?: unknown; type?: unknown };
  if (candidate.type !== "chatThread/createWithId") return false;
  if (typeof candidate.payload !== "object" || candidate.payload === null) {
    return false;
  }
  const payload = candidate.payload as { mode?: unknown };
  return payload.mode === "setup_mcp";
}

function makeThunkDispatch() {
  const innerDispatch = vi.fn<TestDispatch>((action) => action);
  const dispatch = vi.fn<TestDispatch>((action) => {
    if (typeof action === "function") {
      return (action as CapturedThunk)(
        innerDispatch,
        () => ({ config: { lspPort: 0, apiKey: null } }),
        undefined,
      );
    }
    return innerDispatch(action);
  });
  return { dispatch, innerDispatch };
}

describe("buddy UI polish", () => {
  it("actionLabel_humanizes_customization_kind", () => {
    expect(
      actionLabel({
        kind: "draft_customization_change",
        customization_kind: "delegate",
        id: "delegate-a",
        patch: {},
      }),
    ).toBe("Edit delegate");
  });

  it("actionLabel_humanizes_pulse_scope", () => {
    expect(actionLabel({ kind: "create_pulse_report", scope: "all" })).toBe(
      "Create system report",
    );
    expect(actionLabel({ kind: "create_pulse_report", scope: "mcp" })).toBe(
      "Create MCP report",
    );
  });

  it("actionLabel_humanizes_market_kind", () => {
    expect(
      actionLabel({
        kind: "offer_marketplace_install",
        market_kind: "mcp",
        item_id: "github",
      }),
    ).toBe("Install MCP");
  });

  it("shared_navigation_helper_routes_pages", () => {
    const cases: [BuddyPage, string][] = [
      [{ type: "buddy" }, "buddy"],
      [{ type: "stats" }, "stats dashboard"],
      [{ type: "settings" }, "general settings"],
      [{ type: "customization" }, "customization"],
      [{ type: "providers" }, "providers page"],
      [{ type: "default_models" }, "default models"],
      [{ type: "integrations" }, "integrations page"],
      [{ type: "extensions" }, "extensions"],
      [{ type: "marketplace_hub" }, "marketplace hub"],
      [{ type: "marketplace" }, "mcp marketplace"],
      [{ type: "skills_marketplace" }, "skills marketplace"],
      [{ type: "commands_marketplace" }, "commands marketplace"],
      [{ type: "delegates_marketplace" }, "subagents marketplace"],
      [{ type: "tasks_list" }, "tasks list"],
      [{ type: "task_workspace", task_id: "task-a" }, "task workspace"],
      [{ type: "knowledge_graph" }, "knowledge graph"],
      [{ type: "worktrees" }, "tasks list"],
    ];

    for (const [page, expectedName] of cases) {
      const dispatch = vi.fn();
      navigateFromBuddyPage(page, dispatch as never);
      const action = dispatch.mock.calls[0][0] as ReturnType<typeof push>;
      expect(action.payload).toMatchObject({ name: expectedName });
    }
  });

  it("shared_navigation_helper_routes_setup_mode", () => {
    const { dispatch, innerDispatch } = makeThunkDispatch();
    navigateFromBuddyPage(
      { type: "setup_mode", mode: "setup_mcp" },
      dispatch as never,
    );
    const dispatchedActions = innerDispatch.mock.calls.map((call) => call[0]);
    expect(dispatchedActions.some(isSetupModeCreateAction)).toBe(true);
  });

  it("shared_navigation_helper_keeps_worktrees_inside_tasks", () => {
    const dispatch = vi.fn();
    navigateFromBuddyPage({ type: "worktrees" }, dispatch as never);
    const action = dispatch.mock.calls[0][0] as ReturnType<typeof push>;
    expect(action.payload).toEqual({ name: "tasks list" });
  });

  it("BuddyWorld_uses_kit_tooltips_and_focus_ring_for_hotspots", async () => {
    const source = await readGuiSource("features/Buddy/BuddyWorld.tsx");
    const css = await readGuiSource("features/Buddy/BuddyWorld.module.css");

    expect(source).toContain("<Tooltip content={<SceneTip");
    expect(source).not.toMatch(/\btitle=\{/u);
    expect(css).not.toContain(".objectTooltip");
    expect(css).toContain(
      "outline: var(--rf-focus-ring-w) solid var(--rf-focus-ring)",
    );
    expect(css).not.toMatch(/rgba\(255,\s*255,\s*255/u);
  });

  it("BuddyWorld_reschedules_idle_loop_after_noop_branch", async () => {
    const source = await readGuiSource("features/Buddy/BuddyWorld.tsx");

    expect(source).toContain("const [idleTick, setIdleTick]");
    expect(source).toContain("setIdleTick((tick) => tick + 1)");
    expect(source).toContain("activeSpeech,");
    expect(source).toContain("idleTick,");
    expect(source).toContain("reaction,");
    expect(source).toContain("showcaseRun,");
    expect(source).toContain("startShowcase,");
    expect(source).toContain("waypoints,");
  });

  it("BuddyHome_container_renders_split_subcomponents", async () => {
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
      http.get("*/v1/stats/llm/summary", () =>
        HttpResponse.json({
          totals: { total_calls: 1, successful_calls: 1, total_tokens: 10 },
        }),
      ),
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true }),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddyHome />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    expect(screen.getByTestId("buddy-presence")).toBeInTheDocument();
    const world = await screen.findByTestId("buddy-world");
    expect(screen.getByTestId("buddy-world-canvas")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-world-character")).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-summary-strip")).not.toBeInTheDocument();
    expect(screen.getByRole("tablist").compareDocumentPosition(world)).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );
    expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
      "data-compact-bubble",
      "true",
    );

    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Home",
      "Reports",
      "Activity",
      "Play",
      "Settings",
    ]);

    await user.click(screen.getByRole("tab", { name: "Play" }));
    await waitFor(() => {
      expect(screen.getByTestId("buddy-world-character")).toHaveAttribute(
        "data-compact-bubble",
        "false",
      );
    });
    expect(screen.getByTestId("buddy-world")).not.toBe(world);
    expect(screen.getByTestId("buddy-world-canvas")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-summary-strip")).toBeInTheDocument();

    expect(screen.queryByRole("button", { name: "Leave Play" })).toBeNull();
    await user.click(screen.getByRole("tab", { name: "Home" }));
    expect(screen.getByRole("tab", { name: "Home" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getAllByTestId("buddy-world")).toHaveLength(1);
    expect(screen.queryByTestId("buddy-summary-strip")).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Settings" }));
    expect(
      await screen.findByTestId("buddy-personality-panel"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Personality" })).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Activity" }));
    expect(
      await screen.findByTestId("buddy-activity-panel"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("buddy-recent-chats")).toBeInTheDocument();
    expect(
      await screen.findByTestId("buddy-recent-errors-panel"),
    ).toBeInTheDocument();
  });

  it("formatCompactNumber_truncates_decimal_values_below_1000", () => {
    expect(formatCompactNumber(12.9)).toBe("12");
    expect(formatCompactNumber(999.9)).toBe("999");
    expect(formatCompactNumber(-12.9)).toBe("-12");
  });
});
