import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { BuddyAppliedChangesCard } from "../BuddyAppliedChangesCard";
import type { BuddyReceipt } from "../types";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function makeReceipt(overrides?: Partial<BuddyReceipt>): BuddyReceipt {
  return {
    id: "receipt-1",
    action_kind: "apply_config_patch",
    target_path: ".refact/knowledge/handbook/testing.md",
    pre_image: null,
    created_at: "2026-07-03T06:00:00Z",
    undone: false,
    ...overrides,
  };
}

describe("BuddyAppliedChangesCard", () => {
  it("lists an applied change and undoes it", async () => {
    let undoBody: unknown;
    server.use(
      http.get("*/v1/buddy/receipts", () =>
        HttpResponse.json({ receipts: [makeReceipt()] }),
      ),
      http.post("*/v1/buddy/actions/undo", async ({ request }) => {
        undoBody = await request.json();
        return HttpResponse.json({
          receipt: { ...makeReceipt(), undone: true },
        });
      }),
    );
    const { user } = render(<BuddyAppliedChangesCard />, {
      preloadedState: CONFIG_STATE,
    });

    expect(
      await screen.findByTestId("buddy-applied-changes-card"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(".refact/knowledge/handbook/testing.md"),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", {
        name: "Undo .refact/knowledge/handbook/testing.md",
      }),
    );
    await waitFor(() => {
      expect(undoBody).toEqual({ receipt_id: "receipt-1" });
    });
  });

  it("says plainly when nothing has been changed yet", async () => {
    server.use(
      http.get("*/v1/buddy/receipts", () =>
        HttpResponse.json({ receipts: [] }),
      ),
    );
    render(<BuddyAppliedChangesCard />, { preloadedState: CONFIG_STATE });

    expect(
      await screen.findByText("Buddy has not changed any of your files yet."),
    ).toBeInTheDocument();
  });

  it("hides changes that were already undone", async () => {
    server.use(
      http.get("*/v1/buddy/receipts", () =>
        HttpResponse.json({
          receipts: [makeReceipt({ undone: true, undone_at: "2026-07-03" })],
        }),
      ),
    );
    render(<BuddyAppliedChangesCard />, { preloadedState: CONFIG_STATE });

    expect(
      await screen.findByText("Buddy has not changed any of your files yet."),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(".refact/knowledge/handbook/testing.md"),
    ).not.toBeInTheDocument();
  });

  it("reports a failed request instead of claiming nothing was changed", async () => {
    server.use(
      http.get("*/v1/buddy/receipts", () =>
        HttpResponse.json({ detail: "nope" }, { status: 500 }),
      ),
    );
    render(<BuddyAppliedChangesCard />, { preloadedState: CONFIG_STATE });

    expect(
      await screen.findByText("Couldn't load changed files."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
    expect(
      screen.queryByText("Buddy has not changed any of your files yet."),
    ).not.toBeInTheDocument();
  });

  it("surfaces the backend error message when undo fails", async () => {
    server.use(
      http.get("*/v1/buddy/receipts", () =>
        HttpResponse.json({ receipts: [makeReceipt()] }),
      ),
      http.post("*/v1/buddy/actions/undo", () =>
        HttpResponse.json(
          { detail: "action denied by autonomy level" },
          { status: 403 },
        ),
      ),
    );
    const { user } = render(<BuddyAppliedChangesCard />, {
      preloadedState: CONFIG_STATE,
    });

    expect(
      await screen.findByTestId("buddy-applied-changes-card"),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", {
        name: "Undo .refact/knowledge/handbook/testing.md",
      }),
    );
    expect(
      await screen.findByText(
        /Could not undo: .*action denied by autonomy level/,
      ),
    ).toBeInTheDocument();
  });

  it("never re-reads the briefing feed that carried the duplicate opportunity list", async () => {
    let briefingRequested = false;
    server.use(
      http.get("*/v1/buddy/briefing", () => {
        briefingRequested = true;
        return HttpResponse.json({ briefing: null });
      }),
      http.get("*/v1/buddy/receipts", () =>
        HttpResponse.json({ receipts: [makeReceipt()] }),
      ),
    );
    render(<BuddyAppliedChangesCard />, { preloadedState: CONFIG_STATE });

    expect(
      await screen.findByTestId("buddy-applied-changes-card"),
    ).toBeInTheDocument();
    expect(briefingRequested).toBe(false);
    expect(screen.queryByText(/to review/)).not.toBeInTheDocument();
    expect(screen.queryByText("TOP CARDS")).not.toBeInTheDocument();
  });
});
