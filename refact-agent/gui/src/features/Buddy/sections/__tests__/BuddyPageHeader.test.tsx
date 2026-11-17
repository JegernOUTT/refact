import { render, screen, waitFor, within } from "../../../../utils/test-utils";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import { setBuddySnapshot } from "../../buddySlice";
import { BuddyHome } from "../../BuddyHome";

import {
  CONFIG_STATE,
  operatorStatus,
  makePulse,
  makeSnapshot,
} from "../../__tests__/buddyHomeFixtures";

// Existing legacy tests explicitly exercise the disabled compatibility contract.
beforeEach(() => {
  server.use(
    http.get("*/v1/buddy/operator/status", () =>
      HttpResponse.json(operatorStatus("disabled")),
    ),
  );
});

vi.mock("../../BuddyCharacter", async () => {
  const { mockBuddyCharacterModule } = await import(
    "../../__tests__/buddyHomeFixtures"
  );
  return mockBuddyCharacterModule();
});

describe("BuddyHome_layout_contract", () => {
  const layoutHandlers = [
    http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
    http.get("*/v1/stats/llm/summary", () =>
      HttpResponse.json({
        totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
      }),
    ),
    http.get("*/v1/setup/status", () =>
      HttpResponse.json({ configured: true, reasons: [], detail: {} }),
    ),
  ];
  it.each([
    ["enabled", "Enabled"],
    ["disabled", "Local only"],
    ["unavailable", "Not responding"],
  ] as const)(
    "renders the %s work authority branch with its own status pill",
    async (mode, label) => {
      server.use(
        ...layoutHandlers,
        http.get("*/v1/buddy/operator/status", () =>
          HttpResponse.json(operatorStatus(mode)),
        ),
        http.get("*/v1/buddy/opportunities", () =>
          HttpResponse.json({ opportunities: [] }),
        ),
        http.get("*/v1/buddy/work", () =>
          HttpResponse.json({
            schema_version: 1,
            project: operatorStatus("enabled").project,
            data: [],
            next_cursor: null,
          }),
        ),
      );
      const store = setUpStore({ ...CONFIG_STATE });
      store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

      const { user } = render(<BuddyHome />, { store });

      const pill = await screen.findByTestId("buddy-operator-status");
      await waitFor(() => expect(pill).toHaveAttribute("data-state", mode));
      expect(pill).toHaveTextContent(label);

      if (mode === "enabled") {
        await user.click(screen.getByRole("tab", { name: "Reports" }));
        expect(
          await screen.findByText("Nothing needs you right now."),
        ).toBeInTheDocument();
        const desk = screen.getByTestId("buddy-desk");
        expect(
          within(desk).getByRole("heading", { name: "Reports" }),
        ).toBeInTheDocument();
        expect(within(desk).queryByRole("searchbox")).toBeNull();
      }
      const retry = screen.queryByRole("button", {
        name: "Retry connection",
      });
      const check = screen.queryByRole("button", {
        name: "Check connection",
      });
      if (mode === "unavailable") {
        expect(retry).toBeNull();
        expect(check).toBeInTheDocument();
      } else {
        expect(retry).toBeNull();
        expect(check).toBeNull();
      }
    },
  );

  it("renders a broken status pill and an honest check control when status fails", async () => {
    server.use(
      ...layoutHandlers,
      http.get(
        "*/v1/buddy/operator/status",
        () => new HttpResponse(null, { status: 500 }),
      ),
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });

    const pill = await screen.findByTestId("buddy-operator-status");
    await waitFor(() => expect(pill).toHaveAttribute("data-state", "error"));
    expect(pill).toHaveTextContent("Broken");
    expect(
      await screen.findByRole("button", { name: "Check connection" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-opportunities-feed")).toBeNull();
  });
});

describe("BuddyHome_operator_surface", () => {
  const surfaceHandlers = [
    http.get("*/v1/buddy/opportunities", () =>
      HttpResponse.json({ opportunities: [] }),
    ),
    http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
    http.get("*/v1/stats/llm/summary", () =>
      HttpResponse.json({
        totals: { total_calls: 50, successful_calls: 48, total_tokens: 900 },
      }),
    ),
    http.get("*/v1/setup/status", () =>
      HttpResponse.json({ configured: true, reasons: [], detail: {} }),
    ),
  ];
  it("renders the lock holder and invokes the operator reopen endpoint", async () => {
    let reopenCalls = 0;
    const unavailableReason = {
      kind: "already_owned",
      detail:
        "operator store is already owned by another writer (pid 2810136 (refact-lsp 0.1.0) on svakhreev-work, started 2026-09-15 09:36:52 UTC, cmd: refact-lsp --http-port 8001)",
      retry_in_seconds: 900,
    };
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("unavailable", unavailableReason)),
      ),
      http.post("*/v1/buddy/operator/reopen", () => {
        reopenCalls += 1;
        return HttpResponse.json(
          operatorStatus("unavailable", unavailableReason),
        );
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    const banner = await screen.findByTestId("buddy-operator-degraded");
    expect(banner).toHaveTextContent(
      "Another Refact process has the reports open",
    );
    expect(banner).not.toHaveTextContent("pid 2810136");
    await user.click(
      within(banner).getByRole("button", { name: "Technical details" }),
    );
    expect(banner).toHaveTextContent("pid 2810136");
    expect(banner).toHaveTextContent("refact-lsp --http-port 8001");
    expect(banner).toHaveTextContent(
      "Buddy is already trying again in about 15 minutes.",
    );
    await user.click(
      within(banner).getByRole("button", { name: "Retry connection" }),
    );
    await waitFor(() => expect(reopenCalls).toBe(1));
    expect(await within(banner).findByRole("status")).toHaveTextContent(
      "Reports are still unavailable.",
    );
  });

  it("falls back safely for an unfamiliar unavailable reason", async () => {
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(
          operatorStatus("unavailable", {
            kind: "a-new-engine-kind",
            detail: "The engine reported a new condition.",
          }),
        ),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });

    const banner = await screen.findByTestId("buddy-operator-degraded");
    expect(banner).toHaveTextContent("Buddy couldn't open the reports");
    expect(banner).not.toHaveTextContent("a-new-engine-kind");
  });
});
