import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { BuddyBriefingCard } from "../BuddyBriefingCard";
import type { BuddyBriefing } from "../types";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function makeBriefing(overrides?: Partial<BuddyBriefing>): BuddyBriefing {
  return {
    date: "2026-07-03",
    generated_at: "2026-07-03T07:00:00Z",
    job_runs: [
      {
        workflow_id: "refact_error_detective",
        run_count: 3,
        outputs: 1,
        tokens_in: 1200,
        tokens_out: 300,
        last_outcome: null,
      },
    ],
    receipts: [
      {
        id: "receipt-1",
        action_kind: "apply_config_patch",
        target_path: ".refact/knowledge/handbook/testing.md",
        pre_image: null,
        created_at: "2026-07-03T06:00:00Z",
        undone: false,
      },
    ],
    top_cards: [
      {
        id: "card-1",
        summary: "Merge 12 near-duplicate memories",
        priority: "normal",
        confidence: 0.9,
        kind: "memory_ops_batch",
      },
    ],
    pulse: {
      tasks_total: 5,
      memory_pending_ops: 12,
      diagnostics_last_hour: 2,
      git_uncommitted_files: 0,
    },
    spend: {
      day: "2026-07-03",
      llm_calls: 4,
      tokens_in: 1200,
      tokens_out: 300,
    },
    ...overrides,
  };
}

describe("BuddyBriefingCard", () => {
  it("renders nothing without a briefing", async () => {
    server.use(
      http.get("*/v1/buddy/briefing", () =>
        HttpResponse.json({ briefing: null }),
      ),
    );
    render(<BuddyBriefingCard />, { preloadedState: CONFIG_STATE });
    await waitFor(() => {
      expect(
        screen.queryByTestId("buddy-briefing-card"),
      ).not.toBeInTheDocument();
    });
  });

  it("renders top cards, receipts with undo, and spend", async () => {
    let undoBody: unknown;
    server.use(
      http.get("*/v1/buddy/briefing", () =>
        HttpResponse.json({ briefing: makeBriefing() }),
      ),
      http.post("*/v1/buddy/actions/undo", async ({ request }) => {
        undoBody = await request.json();
        return HttpResponse.json({
          receipt: { ...makeBriefing().receipts[0], undone: true },
        });
      }),
    );
    const { user } = render(<BuddyBriefingCard />, {
      preloadedState: CONFIG_STATE,
    });

    expect(
      await screen.findByTestId("buddy-briefing-card"),
    ).toBeInTheDocument();
    expect(screen.getByText("Briefing — 2026-07-03")).toBeInTheDocument();
    expect(
      screen.getByText("Merge 12 near-duplicate memories"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(".refact/knowledge/handbook/testing.md"),
    ).toBeInTheDocument();
    expect(screen.getByText("1 job(s) ran")).toBeInTheDocument();
    expect(screen.getByText("1,500 tokens today")).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", {
        name: "Undo .refact/knowledge/handbook/testing.md",
      }),
    );
    await waitFor(() => {
      expect(undoBody).toEqual({ receipt_id: "receipt-1" });
    });
  });

  it("surfaces the backend error message when undo fails", async () => {
    server.use(
      http.get("*/v1/buddy/briefing", () =>
        HttpResponse.json({ briefing: makeBriefing() }),
      ),
      http.post("*/v1/buddy/actions/undo", () =>
        HttpResponse.json(
          { detail: "action denied by autonomy level" },
          { status: 403 },
        ),
      ),
    );
    const { user } = render(<BuddyBriefingCard />, {
      preloadedState: CONFIG_STATE,
    });

    expect(
      await screen.findByTestId("buddy-briefing-card"),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", {
        name: "Undo .refact/knowledge/handbook/testing.md",
      }),
    );
    expect(
      await screen.findByText(/Undo failed: .*action denied by autonomy level/),
    ).toBeInTheDocument();
  });
});
