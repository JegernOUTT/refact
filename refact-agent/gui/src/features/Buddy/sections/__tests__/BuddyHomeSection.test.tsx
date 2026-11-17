import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../../../utils/test-utils";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import {
  setBuddySnapshot,
  selectHomeSnoozedUntil,
  selectSeenNotificationIds,
} from "../../buddySlice";
import { BuddyHome } from "../../BuddyHome";
import { readBuddyPersistentState, resetBuddyPersistentState } from "../../ui";
import { BUDDY_REPORTS_VIEW_KEY } from "../buddyReportsLink";
import type { BuddyRuntimeEvent } from "../../types";
import {
  CONFIG_STATE,
  operatorStatus,
  makePulse,
  makeSnapshot,
  makeOpportunity,
} from "../../__tests__/buddyHomeFixtures";

// Existing legacy tests explicitly exercise the disabled compatibility contract.
beforeEach(() => {
  server.use(
    http.post("*/v1/buddy/operator/pending-publications", () =>
      HttpResponse.json({
        schema_version: 1,
        project: { project_id: "fixture", canonical_root: "/fixture" },
        data: [],
      }),
    ),
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

describe("BuddyHome_renders_all_sections", () => {
  it("failed hero runtime dismiss remains local and snoozes notifications", async () => {
    let dismissCalled = false;
    const runtime = {
      id: "home-runtime-dismiss-fails",
      signal_type: "chat_error",
      title: "Runtime dismiss fails",
      source: "test",
      status: "failed",
      priority: "high",
      created_at: new Date().toISOString(),
    } satisfies BuddyRuntimeEvent;
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
      http.get("*/v1/stats/llm/summary", () =>
        HttpResponse.json({
          totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
        }),
      ),
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true, reasons: [], detail: {} }),
      ),
      http.post("*/v1/buddy/runtime/:id/dismiss", () => {
        dismissCalled = true;
        return HttpResponse.json({ detail: "offline" }, { status: 503 });
      }),
    );
    const unhandled = vi.fn();
    window.addEventListener("unhandledrejection", unhandled);
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(makeSnapshot(makePulse(), { runtime_queue: [runtime] })),
    );

    try {
      render(<BuddyHome />, { store });
      const world = await screen.findByTestId("buddy-presence");
      const dismissButton = within(world)
        .getAllByRole("button", { hidden: true })
        .find((button) => button.textContent === "Shoo");
      expect(dismissButton).toBeDefined();
      if (!dismissButton) throw new Error("expected dismiss button");
      fireEvent.click(dismissButton);

      await waitFor(() => {
        expect(store.getState().buddy.nowPlaying?.id).toBe(runtime.id);
        expect(store.getState().buddy.nowPlaying?.dismissed).toBe(true);
        expect(dismissCalled).toBe(true);
      });
      expect(selectHomeSnoozedUntil(store.getState())).toBeGreaterThan(
        Date.now(),
      );
      expect(
        `runtime-${runtime.id}` in selectSeenNotificationIds(store.getState()),
      ).toBe(true);
      await new Promise((resolve) => window.setTimeout(resolve, 0));
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("unhandledrejection", unhandled);
      vi.useRealTimers();
    }
  });

  it("failed hero opportunity action stays unseen and prevents duplicate requests", async () => {
    // Opportunity speech intentionally stays silent in some 30-second buckets.
    // Fix only the wall clock to a speaking bucket; keep request timers real.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2024-01-01T12:00:00Z"));
    try {
      const opportunity = makeOpportunity({
        id: "hero-failed-action",
        proposed_actions: [{ kind: "open_page", page: { type: "providers" } }],
      });
      let calls = 0;
      let finish: (() => void) | undefined;
      const responseReady = new Promise<void>((resolve) => {
        finish = resolve;
      });
      server.use(
        http.get("*/v1/buddy/opportunities", () =>
          HttpResponse.json({ opportunities: [opportunity] }),
        ),
        http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
        http.get("*/v1/buddy/briefing", () => HttpResponse.json(null)),
        http.get("*/v1/stats/llm/summary", () =>
          HttpResponse.json({ totals: {} }),
        ),
        http.get("*/v1/setup/status", () =>
          HttpResponse.json({ configured: true }),
        ),
        http.post("*/v1/buddy/opportunities/:id/accept", async () => {
          calls += 1;
          await responseReady;
          return HttpResponse.json(
            { detail: "hero action offline" },
            { status: 503 },
          );
        }),
      );
      const store = setUpStore(CONFIG_STATE);
      store.dispatch(
        setBuddySnapshot(
          makeSnapshot(makePulse(), {
            opportunities: [opportunity],
            runtime_queue: [],
          }),
        ),
      );
      render(<BuddyHome />, { store });
      const world = await screen.findByTestId("buddy-presence");
      const button = within(world)
        .getAllByRole("button", { hidden: true })
        .find((candidate) => candidate.textContent === "Open Providers");
      if (!button) throw new Error("Expected hero opportunity control");
      fireEvent.click(button);
      fireEvent.click(button);
      await waitFor(() => expect(calls).toBe(1));
      finish?.();
      await waitFor(() =>
        expect(store.getState().buddy.activeSpeech?.text).toContain(
          "hero action offline",
        ),
      );
      expect(selectSeenNotificationIds(store.getState())).not.toHaveProperty(
        `opportunity-${opportunity.id}`,
      );
      expect(selectHomeSnoozedUntil(store.getState())).toBeNull();
      expect(calls).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("BuddyHome_operator_surface", () => {
  const operatorProject = operatorStatus("enabled").project;
  const record = (value: unknown) => ({ value, revision: 1, change_seq: 1 });
  const envelope = (data: unknown) => ({
    schema_version: 1,
    project: operatorProject,
    data,
    next_cursor: null,
  });
  const workItem = (id: string, title: string) => ({
    schema_version: 1,
    id,
    project: operatorProject,
    subject: { resource_kind: "legacy_draft", resource_id: id, version: "1" },
    kind: "proposal",
    title,
    state: "needs_attention",
    native_sources: [],
    current_artifact_id: `${id}-report`,
    current_artifact_revision: 1,
    last_material_change_at: "2026-01-01T00:00:00Z",
  });
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
  it("previews at most three saved items on Home without mounting the Desk", async () => {
    const workLimits: string[] = [];
    const items = Array.from({ length: 5 }, (_, index) =>
      record({
        ...workItem(`desk-${index}`, `Saved finding ${index}`),
        problem_summary: `Problem summary ${index}`,
        scope: `scope-${index}`,
      }),
    );
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", ({ request }) => {
        const url = new URL(request.url);
        workLimits.push(
          `${url.searchParams.get("view")}:${url.searchParams.get("limit")}`,
        );
        if (url.searchParams.get("view") === "delivered")
          return HttpResponse.json(envelope([]));
        return HttpResponse.json({
          ...envelope(items),
          next_cursor: 3,
        });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });

    const preview = await screen.findByRole("list", {
      name: "Reports that need you",
    });
    expect(within(preview).getAllByRole("listitem")).toHaveLength(3);
    expect(within(preview).getByText("Problem summary 0")).toBeInTheDocument();
    expect(within(preview).getByText(/scope-0/)).toBeInTheDocument();
    expect(within(preview).queryByText("Saved finding 3")).toBeNull();
    expect(
      within(screen.getByTestId("buddy-home-saved-work")).getByRole("button", {
        name: "View all reports",
      }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-desk")).toBeNull();
    expect(workLimits).toContain("needs_you:3");
    expect(workLimits.some((entry) => entry.endsWith(":50"))).toBe(false);
    expect(
      await screen.findByText("No delivered reports yet."),
    ).toBeInTheDocument();
  });

  it("features a delivered report instead of cancelled runs or housekeeping", async () => {
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", ({ request }) => {
        const url = new URL(request.url);
        if (url.searchParams.get("view") === "delivered")
          return HttpResponse.json(
            envelope([
              record({
                ...workItem("cancelled-run", "Cancelled run"),
                state: "cancelled",
              }),
              record({
                ...workItem("commit-run", "Generated commit message"),
                subject: {
                  resource_kind: "workflow",
                  resource_id: "commit_message",
                  version: "1",
                },
                state: "delivered",
              }),
              record({
                ...workItem("real-report", "Real delivered report"),
                state: "delivered",
              }),
            ]),
          );
        return HttpResponse.json(envelope([]));
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });

    const latest = await screen.findByTestId("buddy-home-latest-report");
    expect(
      await within(latest).findByText("Real delivered report"),
    ).toBeInTheDocument();
    expect(within(latest).queryByText("Cancelled run")).toBeNull();
    expect(within(latest).queryByText("Generated commit message")).toBeNull();
    expect(
      await screen.findByText("Nothing needs you right now."),
    ).toBeInTheDocument();
  });

  it("reports a project mismatch rather than presenting foreign previews as empty", async () => {
    const foreignProject = {
      project_id: "foreign-project",
      canonical_root: "/scratch/foreign-project",
    };
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", ({ request }) => {
        const delivered =
          new URL(request.url).searchParams.get("view") === "delivered";
        return HttpResponse.json({
          ...envelope([
            record({
              ...workItem("foreign-work", "Private foreign report"),
              project: foreignProject,
              state: delivered ? "delivered" : "needs_attention",
            }),
          ]),
          project: foreignProject,
        });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    render(<BuddyHome />, { store });
    expect(
      await screen.findByText("Couldn't load reports."),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("Couldn't load delivered reports."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Private foreign report")).toBeNull();
    expect(screen.queryByText("Nothing needs you right now.")).toBeNull();
  });

  it("keeps native questions on Home when saved work fails to load", async () => {
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get(
        "*/v1/buddy/work",
        () => new HttpResponse(null, { status: 503 }),
      ),
      http.get(
        "*/v1/buddy/decisions/current",
        () => new HttpResponse(null, { status: 503 }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    render(<BuddyHome />, { store });

    const saved = await screen.findByTestId("buddy-home-saved-work");
    expect(
      await within(saved).findByText("Couldn't load reports."),
    ).toBeInTheDocument();
    expect(within(saved).queryByText(/Nothing needs you/)).toBeNull();
    const native = screen.getByTestId("buddy-home-native");
    expect(
      await within(native).findByText("Couldn't load questions."),
    ).toBeInTheDocument();
    expect(screen.getByTestId("buddy-operator-status")).toHaveTextContent(
      "Enabled",
    );
  });

  it("merges simultaneous question and approval failures into one callout with one retry", async () => {
    let questionReads = 0;
    let approvalReads = 0;
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
      http.get("*/v1/buddy/decisions/current", () => {
        questionReads += 1;
        return new HttpResponse(null, { status: 503 });
      }),
      http.get("*/v1/buddy/approvals/current", () => {
        approvalReads += 1;
        return new HttpResponse(null, { status: 503 });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    render(<BuddyHome />, { store });

    const merged = await screen.findByTestId("buddy-home-requests-error");
    expect(merged).toHaveTextContent("Couldn't load questions and approvals.");
    const native = screen.getByTestId("buddy-home-native");
    expect(within(native).getAllByRole("alert")).toHaveLength(1);
    const seen = [questionReads, approvalReads];
    fireEvent.click(within(merged).getByRole("button", { name: "Retry" }));
    await waitFor(() => {
      expect(questionReads).toBeGreaterThan(seen[0]);
      expect(approvalReads).toBeGreaterThan(seen[1]);
    });
  });

  it("opens Reports on the matching view from the Home cards", async () => {
    resetBuddyPersistentState();
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    render(<BuddyHome />, { store });

    const latest = await screen.findByTestId("buddy-home-latest-report");
    fireEvent.click(
      within(latest).getByRole("button", { name: "View delivered" }),
    );
    expect(readBuddyPersistentState(BUDDY_REPORTS_VIEW_KEY)).toBe("delivered");
    expect(screen.getByRole("tab", { name: "Reports" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });
});
