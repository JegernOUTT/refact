import { render, screen, waitFor, within } from "../../../../utils/test-utils";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import {
  setBuddySnapshot,
  setPulse,
  addBuddySuggestion,
} from "../../buddySlice";
import { push } from "../../../Pages/pagesSlice";
import { BuddyPulseCard } from "../../BuddyPulseCard";
import { BuddyOpportunityCard } from "../../BuddyOpportunityCard";
import { BuddyOpportunitiesFeed } from "../../BuddyOpportunitiesFeed";
import { BuddyHome } from "../../BuddyHome";
import {
  buddyOperatorApi,
  buddyOperatorIdentity,
} from "../../../../services/refact/buddyOperator";
import {
  CONFIG_STATE,
  operatorStatus,
  makePulse,
  makeSnapshot,
  makeOpportunity,
} from "../../__tests__/buddyHomeFixtures";
import { makeSuggestion } from "../../__tests__/buddyHomeFixtures";

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

describe("BuddyHome single work authority", () => {
  it("opens the durable proposal when its legacy editing draft has expired", async () => {
    const project = operatorStatus("enabled").project;
    const work = {
      schema_version: 1,
      id: "saved-proposal",
      project,
      subject: {
        resource_kind: "legacy_draft",
        resource_id: "expired",
        version: "1",
      },
      kind: "proposal",
      title: "Preserved proposal",
      state: "needs_attention",
      native_sources: [],
      current_artifact_id: "saved-report",
      current_artifact_revision: 1,
      last_material_change_at: "2026-01-01T00:00:00Z",
    };
    const record = (value: unknown) => ({ value, revision: 1, change_seq: 1 });
    const envelope = (data: unknown) => ({
      schema_version: 1,
      project,
      data,
      next_cursor: null,
    });
    let legacyRequests = 0;
    let resolved = 0;
    server.use(
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/operator/legacy-drafts/expired/work", () => {
        resolved++;
        return HttpResponse.json(envelope(record(work)));
      }),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
      http.get("*/v1/buddy/work/saved-proposal", () =>
        HttpResponse.json(envelope(record(work))),
      ),
      http.get("*/v1/buddy/artifacts/saved-report", () =>
        HttpResponse.json(
          envelope(
            record({
              schema_version: 1,
              artifact_id: "saved-report",
              revision: 1,
              project,
              source_run_ids: [],
              title: "Retained proposal report",
              conclusion: "The proposal still needs a decision.",
              delta: "",
              body: "Durable content survives the editing session.",
              uncertainty: null,
              proposed_action_ids: [],
              evidence: [],
              provenance: {
                source_run_ids: [],
                evidence_refs: [],
                contains_untrusted_source: false,
              },
            }),
          ),
        ),
      ),
      http.get("*/v1/buddy/work/saved-proposal/read", () =>
        HttpResponse.json(envelope(null)),
      ),
      http.get("*/v1/buddy/work/saved-proposal/snooze", () =>
        HttpResponse.json(envelope(null)),
      ),
      http.get("*/v1/buddy/work/saved-proposal/actions", () =>
        HttpResponse.json(envelope([])),
      ),
      http.get("*/v1/buddy/work/saved-proposal/outcomes", () =>
        HttpResponse.json(envelope([])),
      ),
      http.get("*/v1/buddy/artifacts/saved-report/revisions", () =>
        HttpResponse.json(envelope([])),
      ),
      http.all("*/v1/buddy/drafts/expired", () => {
        legacyRequests++;
        return new HttpResponse(null, { status: 404 });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    store.dispatch(push({ name: "buddy", draftId: "expired" }));
    await Promise.all([
      store
        .dispatch(
          buddyOperatorApi.endpoints.getBuddyLegacyDraftWork.initiate({
            id: "expired",
            project_id: "fixture",
            endpoint_identity: buddyOperatorIdentity(store.getState()),
          }),
        )
        .unwrap(),
      store
        .dispatch(
          buddyOperatorApi.endpoints.getBuddyOperatorArtifact.initiate({
            id: "saved-report",
            revision: 1,
            project_id: "fixture",
            endpoint_identity: buddyOperatorIdentity(store.getState()),
          }),
        )
        .unwrap(),
    ]);
    render(<BuddyHome />, { store });
    expect(
      await screen.findByText("Retained proposal report"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Durable content survives the editing session."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("The proposal still needs a decision."),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Dismiss draft" }),
    ).not.toBeInTheDocument();
    expect(resolved).toBe(1);
    expect(legacyRequests).toBe(0);
  });
});

describe("BuddyPulseCard_renders_pulse", () => {
  it("shows pulse counts from store", async () => {
    const pulse = makePulse();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setPulse(pulse));

    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );

    render(<BuddyPulseCard />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    await waitFor(() => {
      expect(screen.getByTestId("buddy-pulse-card")).toBeInTheDocument();
    });
    expect(screen.getByText(/3 open/)).toBeInTheDocument();
    expect(screen.getByText(/1 stuck/)).toBeInTheDocument();
    expect(screen.getByText(/2 abandoned/)).toBeInTheDocument();
    expect(screen.getByText(/7 in last hour/)).toBeInTheDocument();
    expect(screen.getByText(/5 files/)).toBeInTheDocument();
  });
});

describe("BuddyOpportunityCard_dismiss_calls_mutation", () => {
  it("clicking Dismiss calls dismiss mutation", async () => {
    let dismissed = false;
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.post("*/v1/buddy/opportunities/:id/dismiss", () => {
        dismissed = true;
        return HttpResponse.json({ snapshot: makeSnapshot() });
      }),
    );

    const opp = makeOpportunity({
      id: "opp-dismiss-1",
      proposed_actions: [{ kind: "dismiss" }],
    });

    const { user } = render(<BuddyOpportunityCard opportunity={opp} />, {
      preloadedState: CONFIG_STATE,
    });

    const dismissBtn = screen.getByText("Dismiss");
    await user.click(dismissBtn);

    await waitFor(() => {
      expect(dismissed).toBe(true);
    });
  });
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
  it("shows the opportunity list exactly once", async () => {
    const opportunity = makeOpportunity({ id: "single-surface" });
    server.use(
      ...layoutHandlers,
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [opportunity] }),
      ),
      http.get("*/v1/buddy/briefing", () =>
        HttpResponse.json({
          date: "2024-01-01",
          top_cards: [
            {
              id: opportunity.id,
              priority: "high",
              summary: opportunity.summary,
            },
          ],
          receipts: [],
          job_runs: [],
          spend: { tokens_in: 1, tokens_out: 1 },
          pulse: { diagnostics_last_hour: 0 },
        }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(makePulse(), { opportunities: [opportunity] }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });

    await screen.findByTestId("buddy-home-content");
    await user.click(screen.getByRole("tab", { name: "Reports" }));

    const feeds = await screen.findAllByTestId("buddy-opportunities-feed");
    expect(feeds).toHaveLength(1);
    expect(
      within(feeds[0]).getByText("Model config is broken"),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-briefing-card")).toBeNull();
  });
});

describe("BuddyOpportunitiesFeed_suggestions", () => {
  it("shows active Buddy suggestions when detector opportunities are empty", async () => {
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    store.dispatch(addBuddySuggestion(makeSuggestion()));

    render(<BuddyOpportunitiesFeed />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    await waitFor(() => {
      expect(screen.getByText("Warm up this workspace")).toBeInTheDocument();
    });
  });
});
