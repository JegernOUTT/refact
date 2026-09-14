import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  within,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BuddyDesk } from "./BuddyDesk";
import type {
  ActionExecution,
  ArtifactRevision,
  RunOutcome,
  RunStatus,
  WorkItem,
  WorkItemState,
} from "./operatorTypes";
import type { DeskAction } from "../../services/refact/buddyOperator";

const project = { project_id: "project", canonical_root: "/scratch/project" };
const config = {
  config: { lspPort: 8001, host: "vscode" as const, themeProps: {} },
};
const item: WorkItem = {
  schema_version: 1,
  id: "work-1",
  project,
  subject: {
    resource_kind: "check",
    resource_id: "provider health",
    version: "1",
  },
  kind: "diagnosis",
  title: "Provider setup diagnosis",
  state: "needs_attention",
  native_sources: [],
  current_artifact_id: "artifact-1",
  current_artifact_revision: 1,
  last_material_change_at: "2026-09-05T00:00:00Z",
};
const artifact: ArtifactRevision = {
  schema_version: 1,
  artifact_id: "artifact-1",
  revision: 1,
  project,
  source_run_ids: ["run-1"],
  title: "Connection assessment",
  conclusion: "The token has expired.",
  delta: "Authentication stopped working.",
  body: "Reconnect the provider from its settings page.",
  uncertainty: "Network access was not tested.",
  proposed_action_ids: ["reconnect"],
  evidence: [],
  provenance: {
    source_run_ids: ["run-1"],
    evidence_refs: [],
    contains_untrusted_source: false,
  },
};
const record = <T,>(value: T) => ({ value, revision: 1, change_seq: 1 });
const envelope = <T,>(data: T, next_cursor: number | null = null) => ({
  schema_version: 1,
  project,
  data,
  next_cursor,
});

const workStateMatrix: Record<WorkItemState, { view: string; label: string }> =
  {
    needs_attention: { view: "Needs you", label: "Needs attention" },
    working: { view: "Working on", label: "Working" },
    delivered: { view: "Delivered", label: "Delivered" },
    watching: { view: "Watching", label: "Watching" },
    blocked: { view: "Needs you", label: "Blocked" },
    cancelled: { view: "Delivered", label: "Cancelled" },
  };
const runStatusMatrix: Record<
  RunStatus,
  { state: WorkItemState; label: string }
> = {
  completed: { state: "delivered", label: "Completed" },
  no_change: { state: "watching", label: "No change" },
  partial: { state: "needs_attention", label: "Partial" },
  inconclusive: { state: "needs_attention", label: "Inconclusive" },
  blocked: { state: "blocked", label: "Blocked" },
  failed: { state: "blocked", label: "Failed" },
  cancelled: { state: "cancelled", label: "Cancelled" },
};

describe("BuddyDesk", () => {
  beforeEach(() => {
    server.use(
      http.get("*/v1/buddy/work/:id/snooze", () =>
        HttpResponse.json(envelope(null)),
      ),
      http.get("*/v1/buddy/work/:id/read", () =>
        HttpResponse.json(envelope(null)),
      ),
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([])),
      ),
    );
  });
  it.each([
    ["Needs you", "needs_you"],
    ["Working on", "working_on"],
    ["Delivered", "delivered"],
    ["Watching", "watching"],
  ])(
    "places only matching work states in %s and retains honest labels",
    async (view, query) => {
      const requests: (string | null)[] = [];
      const states = Object.keys(workStateMatrix) as WorkItemState[];
      server.use(
        http.get("*/v1/buddy/work", ({ request }) => {
          requests.push(new URL(request.url).searchParams.get("view"));
          return HttpResponse.json(
            envelope(
              states.map((state) =>
                record({
                  ...item,
                  id: `work-${state}`,
                  title: `Outcome ${state}`,
                  state,
                  current_artifact_id: null,
                  current_artifact_revision: null,
                }),
              ),
            ),
          );
        }),
      );
      render(<BuddyDesk />, { preloadedState: config });
      await screen.findByRole("button", { name: /Outcome needs_attention/ });
      fireEvent.click(screen.getByRole("button", { name: view }));
      await waitFor(() => expect(requests).toContain(query));
      for (const state of states) {
        const expected = workStateMatrix[state];
        const rowName = new RegExp(`Outcome ${state}`);
        if (expected.view !== view) {
          expect(screen.queryByRole("button", { name: rowName })).toBeNull();
          continue;
        }
        const row = await screen.findByRole("button", { name: rowName });
        expect(
          within(row).getByText(expected.label, { exact: true }),
        ).toBeTruthy();
        if (state === "cancelled") {
          expect(
            within(row).queryByText("Delivered", { exact: true }),
          ).toBeNull();
        }
        fireEvent.click(row);
        expect(
          within(
            screen.getByRole("article", { name: "Work detail" }),
          ).getByText(`${expected.label} · ${item.subject.resource_id}`),
        ).toBeTruthy();
        fireEvent.click(
          screen.getByRole("button", { name: "Back to work list" }),
        );
      }
      expect(screen.getByRole("button", { name: view })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    },
  );

  it.each(Object.keys(runStatusMatrix) as RunStatus[])(
    "renders %s through recorded-check history with coverage, not verification",
    async (status) => {
      const expected = runStatusMatrix[status];
      const run: RunOutcome = {
        schema_version: 1,
        run_id: `run-${status}`,
        work_id: item.id,
        parent_run_id: null,
        project,
        worker: { worker_id: "worker", engine_instance_id: "engine", epoch: 1 },
        origin: "autonomous",
        workflow: null,
        input_fingerprint: "saved-input",
        coverage: `Observed scope for ${status}; runtime behavior was not tested.`,
        started_at: "2026-09-05T00:00:00Z",
        ended_at: "2026-09-05T00:00:01Z",
        status,
        evidence: [],
        effects: [],
        failure_class: null,
      };
      const historyRequests: string[] = [];
      const outcomeRequests: string[] = [];
      server.use(
        http.get("*/v1/buddy/work", () =>
          HttpResponse.json(
            envelope([record({ ...item, state: expected.state })]),
          ),
        ),
        http.get("*/v1/buddy/artifacts/:id", () =>
          HttpResponse.json(envelope(record(artifact))),
        ),
        http.get("*/v1/buddy/work/:id/outcomes", ({ request, params }) => {
          expect(params.id).toBe(item.id);
          expect(new URL(request.url).searchParams.get("project_id")).toBe(
            project.project_id,
          );
          historyRequests.push(request.url);
          return HttpResponse.json(envelope([record(run)]));
        }),
        http.get("*/v1/buddy/outcomes/:id", ({ request, params }) => {
          expect(params.id).toBe(run.run_id);
          expect(new URL(request.url).searchParams.get("project_id")).toBe(
            project.project_id,
          );
          outcomeRequests.push(request.url);
          return HttpResponse.json(envelope(record(run)));
        }),
      );
      render(<BuddyDesk />, { preloadedState: config });
      const workStatus = workStateMatrix[expected.state];
      fireEvent.click(screen.getByRole("button", { name: workStatus.view }));
      fireEvent.click(
        await screen.findByRole("button", { name: /Provider setup diagnosis/ }),
      );
      await screen.findByText("The token has expired.");
      expect(historyRequests).toHaveLength(0);
      expect(outcomeRequests).toHaveLength(0);
      const details = screen
        .getByText("All recorded checks, including unchanged runs")
        .closest("details");
      if (!details) throw new Error("recorded-check disclosure missing");
      details.open = true;
      fireEvent(details, new Event("toggle"));
      const row = await within(details).findByRole("button", {
        name: `${expected.label} · ${run.ended_at} · ${run.run_id}`,
      });
      expect(historyRequests).toHaveLength(1);
      expect(outcomeRequests).toHaveLength(0);
      expect(details).not.toHaveTextContent(/verified/i);
      fireEvent.click(row);
      const evidence = within(
        screen.getByRole("region", { name: "Recorded run" }),
      );
      expect(await evidence.findByText(run.coverage)).toBeTruthy();
      expect(
        evidence.getByText(
          `${expected.label} · ${run.started_at} — ${run.ended_at}`,
        ),
      ).toBeTruthy();
      expect(
        evidence.getByRole("heading", { name: "Observed scope" }),
      ).toBeTruthy();
      expect(
        evidence.getByText("No resource effects were recorded."),
      ).toBeTruthy();
      expect(evidence.queryByText(/verified/i)).toBeNull();
      expect(outcomeRequests).toHaveLength(1);
    },
  );

  it("snoozes independently of read status and sends attention filters to the server", async () => {
    const writes: Record<string, unknown>[] = [];
    const filters: string[] = [];
    server.use(
      http.get("*/v1/buddy/work", ({ request }) => {
        filters.push(
          new URL(request.url).searchParams.get("attention") ?? "all",
        );
        return HttpResponse.json(envelope([record(item)]));
      }),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.post("*/v1/buddy/work/:id/snooze", async ({ request }) => {
        const body = (await request.json()) as Record<string, unknown>;
        writes.push(body);
        return HttpResponse.json(
          envelope(
            body.minutes === null
              ? null
              : {
                  schema_version: 1,
                  project,
                  work_id: item.id,
                  artifact_id: artifact.artifact_id,
                  artifact_revision: 1,
                  until: "2099-01-01T00:00:00Z",
                },
          ),
        );
      }),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: /Provider setup diagnosis/ }),
    );
    const snooze = await screen.findByRole("button", {
      name: "Snooze for one hour",
    });
    await waitFor(() => expect(snooze).toBeEnabled());
    fireEvent.click(snooze);
    fireEvent.click(snooze);
    expect(await screen.findByText(/Snoozed until/)).toBeTruthy();
    expect(writes).toHaveLength(1);
    expect(writes[0]).toMatchObject({
      project_id: project.project_id,
      canonical_root: project.canonical_root,
      minutes: 60,
      artifact_revision: 1,
    });
    expect(
      screen.getByRole("button", { name: "Mark this revision read" }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "End snooze" }));
    expect(
      await screen.findByRole("button", { name: "Snooze for one hour" }),
    ).toBeTruthy();
    expect(writes[1].minutes).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Back to work list" }));
    fireEvent.change(
      screen.getByRole("combobox", { name: "Report attention" }),
      { target: { value: "unread" } },
    );
    await waitFor(() => expect(filters).toContain("unread"));
  });
  it("loads a real API-shaped report without navigating to a worker chat", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", ({ request }) => {
        expect(new URL(request.url).searchParams.get("revision")).toBe("1");
        return HttpResponse.json(envelope(record(artifact)));
      }),
    );
    const { store } = render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
    });
    fireEvent.click(row);
    expect(await screen.findByText("The token has expired.")).toBeTruthy();
    expect(screen.getByText("Network access was not tested.")).toBeTruthy();
    expect(
      screen.getByText(/no proposed fix is applied automatically/),
    ).toBeTruthy();
    expect(store.getState().pages.at(-1)?.name).toBe("login page");
    fireEvent.click(screen.getByRole("button", { name: "Back to work list" }));
    await waitFor(() => expect(document.activeElement).toBe(row));
  });

  it("collapses to one honest line instead of empty desk chrome", async () => {
    server.use(
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
    );
    render(<BuddyDesk />, { preloadedState: config });
    expect(
      await screen.findByText("Nothing needs you right now."),
    ).toBeTruthy();
    const desk = screen.getByRole("region", { name: "Buddy Desk" });
    expect(
      within(desk).queryByRole("heading", { name: "Buddy Desk" }),
    ).toBeNull();
    expect(within(desk).queryByRole("searchbox")).toBeNull();
    expect(
      within(desk).queryByRole("combobox", { name: "Report attention" }),
    ).toBeNull();
    expect(
      within(desk).queryByRole("button", { name: "Needs you" }),
    ).toBeNull();
    expect(within(desk).queryByText(/Pick something on the left/)).toBeNull();
    fireEvent.click(
      within(desk).getByRole("button", { name: "Browse saved work" }),
    );
    expect(
      await screen.findByRole("button", { name: "Needs you" }),
    ).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Buddy Desk" })).toBeTruthy();
    expect(screen.queryByRole("searchbox")).toBeNull();
  });

  it("keeps the desk expanded for a deep link even when the list is empty", async () => {
    server.use(
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
      http.get("*/v1/buddy/work/work-1", () =>
        HttpResponse.json(envelope(record(item))),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    render(<BuddyDesk workId={item.id} projectId={project.project_id} />, {
      preloadedState: config,
    });
    expect(await screen.findByText("The token has expired.")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Buddy Desk" })).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "Browse saved work" }),
    ).toBeNull();
  });

  it("reports a failed request rather than collapsing as if there were no work", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json({ detail: "unavailable" }, { status: 503 }),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Buddy could not reach your saved work.",
    );
    expect(screen.queryByText("Nothing needs you right now.")).toBeNull();
    expect(screen.getByRole("heading", { name: "Buddy Desk" })).toBeTruthy();
    expect(screen.queryByRole("searchbox")).toBeNull();
  });

  it("keeps internal resource identifiers out of the work rows", async () => {
    const digest = `producer:${"a1b2c3d4".repeat(8)}`;
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope([
            record({
              ...item,
              subject: { ...item.subject, resource_id: digest },
            }),
          ]),
        ),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
    });
    expect(row).not.toHaveTextContent(digest);
    expect(row).toHaveTextContent("Needs attention");
    fireEvent.click(row);
    const detail = within(screen.getByRole("article", { name: "Work detail" }));
    expect(detail.getByText("Needs attention")).toBeTruthy();
    expect(screen.queryByText(new RegExp(digest))).toBeNull();
  });

  it("does not offer a report placeholder when the current view has nothing to pick", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record({ ...item, state: "delivered" })])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(await screen.findByRole("button", { name: "Delivered" }));
    expect(
      await screen.findByRole("button", { name: /Provider setup diagnosis/ }),
    ).toBeTruthy();
    expect(screen.getByText(/Pick something on the left/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Watching" }));
    expect(
      await screen.findByText("Nothing is being watched right now."),
    ).toBeTruthy();
    expect(screen.queryByText(/Pick something on the left/)).toBeNull();
  });

  it("shows unavailable rather than an empty successful result", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json({ detail: "disabled" }, { status: 503 }),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.queryByText(/No needs you items/)).toBeNull();
    expect(screen.getByRole("button", { name: "Retry work" })).toBeTruthy();
  });

  it("loads a linked work item even when it is outside the loaded list page", async () => {
    server.use(
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
      http.get("*/v1/buddy/work/work-1", ({ request }) => {
        expect(new URL(request.url).searchParams.get("project_id")).toBe(
          project.project_id,
        );
        return HttpResponse.json(envelope(record(item)));
      }),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    render(<BuddyDesk workId={item.id} projectId={project.project_id} />, {
      preloadedState: config,
    });
    expect(await screen.findByText("The token has expired.")).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: /Provider setup diagnosis/ }),
    ).toBeNull();
  });

  it("refuses a deep-link response with a different work identity", async () => {
    server.use(
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
      http.get("*/v1/buddy/work/work-1", () =>
        HttpResponse.json(envelope(record({ ...item, id: "other" }))),
      ),
    );
    render(<BuddyDesk workId={item.id} projectId={project.project_id} />, {
      preloadedState: config,
    });
    expect(
      await screen.findByText(/No other report was substituted/),
    ).toBeTruthy();
    expect(screen.queryByText("The token has expired.")).toBeNull();
  });

  it("loads historical report text on demand without giving old revisions runnable actions", async () => {
    let reads = 0;
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.get("*/v1/buddy/artifacts/:id/revisions", () => {
        reads += 1;
        return HttpResponse.json(
          envelope([
            record({
              ...artifact,
              title: "Earlier assessment",
              body: "Earlier saved evidence.",
            }),
          ]),
        );
      }),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: /Provider setup diagnosis/ }),
    );
    await screen.findByText("The token has expired.");
    expect(reads).toBe(0);
    const details = screen
      .getByText("Saved report revisions")
      .closest("details");
    expect(details).not.toBeNull();
    if (!details) throw new Error("history disclosure missing");
    details.open = true;
    fireEvent(details, new Event("toggle"));
    expect(
      await screen.findByText("Revision 1 · Earlier assessment"),
    ).toBeTruthy();
    expect(
      screen.getByText(
        "Historical evidence only. Actions are available on the current revision.",
      ),
    ).toBeTruthy();
    expect(reads).toBe(1);
  });

  it("marks only the displayed revision read after acknowledgment and keeps the work unresolved", async () => {
    let saved: unknown = null;
    let calls = 0;
    let release: (() => void) | undefined;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.get("*/v1/buddy/work/:id/read", () =>
        HttpResponse.json(envelope(saved)),
      ),
      http.post("*/v1/buddy/work/:id/read", async ({ request }) => {
        calls += 1;
        expect(await request.json()).toEqual({
          project_id: project.project_id,
          artifact_id: artifact.artifact_id,
          artifact_revision: artifact.revision,
        });
        await pending;
        saved = {
          schema_version: 1,
          project,
          work_id: item.id,
          artifact_id: artifact.artifact_id,
          last_read_revision: 1,
          read_at: "2026-09-05T00:00:01Z",
          snoozed_until: null,
          muted: false,
        };
        return HttpResponse.json(envelope(saved));
      }),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: /Provider setup diagnosis/ }),
    );
    const mark = await screen.findByRole("button", {
      name: "Mark this revision read",
    });
    await waitFor(() => expect(mark).toBeEnabled());
    expect(calls).toBe(0);
    fireEvent.click(mark);
    fireEvent.click(mark);
    await waitFor(() => expect(calls).toBe(1));
    expect(
      screen.queryByText("This report revision is marked read."),
    ).toBeNull();
    release?.();
    expect(
      await screen.findByText("This report revision is marked read."),
    ).toBeTruthy();
    expect(
      screen.getByText(
        "Reading does not accept a proposal or resolve this work.",
      ),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Back to work list" }));
    fireEvent.click(
      screen.getByRole("button", { name: /Provider setup diagnosis/ }),
    );
    expect(
      await screen.findByText("This report revision is marked read."),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /Provider setup diagnosis/ }),
    ).toHaveTextContent("Needs attention");
    expect(calls).toBe(1);
  });

  it("does not claim read status when the backend rejects a stale revision", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.post("*/v1/buddy/work/:id/read", () =>
        HttpResponse.json({ detail: "report changed" }, { status: 409 }),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: /Provider setup diagnosis/ }),
    );
    const mark = await screen.findByRole("button", {
      name: "Mark this revision read",
    });
    await waitFor(() => expect(mark).toBeEnabled());
    fireEvent.click(mark);
    expect(
      await screen.findByText(/Reading status was not acknowledged/),
    ).toBeTruthy();
    expect(
      screen.queryByText("This report revision is marked read."),
    ).toBeNull();
  });

  it("refuses records belonging to another project instead of displaying them", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope([
            record({
              ...item,
              title: "Other project's private work",
              project: {
                project_id: "other",
                canonical_root: "/scratch/other",
              },
            }),
          ]),
        ),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.queryByText("Other project's private work")).toBeNull();
  });

  it("loads more without replacing prior work and searches saved work through the backend", async () => {
    server.use(
      http.get("*/v1/buddy/work", ({ request }) => {
        const cursor = new URL(request.url).searchParams.get("cursor");
        const search = new URL(request.url).searchParams.get("search");
        expect(new URL(request.url).searchParams.get("view")).toBe("needs_you");
        return HttpResponse.json(
          cursor || search === "Second"
            ? envelope([
                record({ ...item, id: "work-2", title: "Second diagnosis" }),
              ])
            : envelope([record(item)], 1),
        );
      }),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: "Load more work" }),
    );
    expect(
      await screen.findByRole("button", { name: /Second diagnosis/ }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /Provider setup diagnosis/ }),
    ).toBeTruthy();
    fireEvent.change(screen.getByRole("searchbox"), {
      target: { value: "Second" },
    });
    expect(
      screen.queryByRole("button", { name: /Provider setup diagnosis/ }),
    ).toBeNull();
    expect(
      await screen.findByRole("button", { name: /Second diagnosis/ }),
    ).toBeTruthy();
  });

  const action: DeskAction = {
    spec: {
      schema_version: 1,
      action_id: "check_saved_evidence",
      kind: "check_saved_evidence",
      adapter: "operator.saved_evidence",
      project,
      resource: {
        resource_kind: "saved_report",
        resource_id: item.id,
        version: "digest",
      },
      expected_version: "digest",
      parameter_ref: null,
      availability: "available",
      confirmation: "none",
      authority_class: "read_only",
      capabilities: {
        can_preview: true,
        can_execute: true,
        can_verify: false,
        can_recover: false,
        can_cancel: false,
      },
    },
    spec_hash: "bound-hash",
    work_revision: 1,
    artifact_id: artifact.artifact_id,
    artifact_revision: 1,
    label: "Check saved evidence links",
    description: "Only retained metadata is checked. No fix is applied.",
    latest_execution: null,
  };
  const receipt = (key: string): ActionExecution => ({
    schema_version: 1,
    execution_id: "execution-1",
    project,
    worker: { worker_id: "worker", engine_instance_id: "engine", epoch: 1 },
    idempotency_key: key,
    action_id: action.spec.action_id,
    spec_hash: action.spec_hash,
    requester: {
      source_kind: "check",
      source_id: "buddy-desk:saved-evidence",
      source_version: "1",
    },
    decision: null,
    status: "partial",
    started_at: "2026-09-05T00:00:00Z",
    ended_at: "2026-09-05T00:00:01Z",
    effects: [],
    verification_evidence: [
      "Evidence metadata matches: 0/0.",
      "Findings are not independently verified.",
    ],
    recovery: null,
  });
  function setupActions(getAction: () => DeskAction = () => action) {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([getAction()])),
      ),
    );
  }
  async function openReport() {
    fireEvent.click(
      await screen.findByRole("button", { name: /Provider setup diagnosis/ }),
    );
    return screen.findByRole("button", { name: "Check saved evidence links" });
  }

  it("executes only the server-registered action once on doubleclick and displays its saved partial receipt", async () => {
    let saved: ActionExecution | null = null;
    setupActions(() => ({
      ...action,
      latest_execution: saved ? record(saved) : null,
    }));
    const requests: Record<string, unknown>[] = [];
    let release: (() => void) | undefined;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    server.use(
      http.post(
        "*/v1/buddy/work/:id/actions/:action/execute",
        async ({ request, params }) => {
          expect(params.action).toBe("check_saved_evidence");
          const body = (await request.json()) as Record<string, unknown>;
          requests.push(body);
          await pending;
          saved = receipt(body.idempotency_key as string);
          return HttpResponse.json(envelope(record(saved)));
        },
      ),
    );
    const { store } = render(<BuddyDesk />, { preloadedState: config });
    const button = await openReport();
    fireEvent.click(button);
    fireEvent.click(button);
    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual({
      project_id: project.project_id,
      work_revision: 1,
      artifact_id: artifact.artifact_id,
      artifact_revision: 1,
      spec_hash: action.spec_hash,
      idempotency_key: requests[0].idempotency_key,
    });
    expect(typeof requests[0].idempotency_key).toBe("string");
    expect(screen.queryByText("Saved check: Partial")).toBeNull();
    release?.();
    expect(await screen.findByText("Saved check: Partial")).toBeTruthy();
    expect(
      screen.getByText("Findings are not independently verified."),
    ).toBeTruthy();
    expect(store.getState().pages.at(-1)?.name).toBe("login page");
    fireEvent.click(screen.getByRole("button", { name: "Back to work list" }));
    await openReport();
    expect(await screen.findByText("Saved check: Partial")).toBeTruthy();
    expect(requests).toHaveLength(1);
  });

  it("retains request identity after a lost response and does not invent success", async () => {
    setupActions();
    const keys: string[] = [];
    server.use(
      http.post(
        "*/v1/buddy/work/:id/actions/:action/execute",
        async ({ request }) => {
          const body = (await request.json()) as { idempotency_key: string };
          keys.push(body.idempotency_key);
          if (keys.length === 1)
            return HttpResponse.json(
              { detail: "unavailable" },
              { status: 503 },
            );
          return HttpResponse.json(
            envelope(record(receipt(body.idempotency_key))),
          );
        },
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(await openReport());
    expect(
      await screen.findByText(/No acknowledged result was received/),
    ).toBeTruthy();
    expect(screen.queryByText("Saved check: Partial")).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: "Check saved evidence links" }),
    );
    expect(await screen.findByText("Saved check: Partial")).toBeTruthy();
    expect(keys).toHaveLength(2);
    expect(keys[0]).toBe(keys[1]);
  });

  it("does not enable an unsupported action or one for an old report revision", async () => {
    setupActions(() => ({
      ...action,
      spec: { ...action.spec, authority_class: "mutating" },
    }));
    render(<BuddyDesk />, { preloadedState: config });
    expect(await openReport()).toBeDisabled();
    expect(screen.getByText(/not supported by this client/)).toBeTruthy();
    server.use(
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([{ ...action, artifact_revision: 2 }])),
      ),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Reload action status" }),
    );
    expect(
      await screen.findByText(
        /No registered actions are available for this report revision/,
      ),
    ).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "Check saved evidence links" }),
    ).toBeNull();
  });

  it.each([
    ["extensions", "markdown_project_apply", "native_markdown"],
    ["candidate_artifact", "stage_exact_artifact", "candidate_artifact"],
    [
      "candidate_artifact_cleanup",
      "remove_exact_orphan_artifacts",
      "candidate_artifact_cleanup",
    ],
  ])(
    "routes registered %s proposals with separate work/resource identities to one exact preview",
    async (adapter, kind, resourceKind) => {
      const spec = {
        ...action.spec,
        adapter,
        kind,
        authority_class: "mutating" as const,
        confirmation: "user_confirmation_required" as const,
        parameter_ref: "durable-proposal-not-editor-session",
        resource: {
          resource_kind: resourceKind,
          resource_id: "target-not-work-id",
          version: "digest",
        },
      };
      setupActions(() => ({ ...action, work_id: item.id, spec }));
      const previews: unknown[] = [];
      server.use(
        http.post("*/v1/buddy/actions/preview", async ({ request }) => {
          previews.push(await request.json());
          return HttpResponse.json(
            envelope({
              resource_id: spec.resource.resource_id,
              expected_version: "digest",
              proposed_version: "next",
              reload_needed: false,
              diff: "Saved durable proposal",
            }),
          );
        }),
        http.post("*/v1/buddy/actions/rights", () =>
          HttpResponse.json(envelope(null)),
        ),
        http.post("*/v1/buddy/actions/prepare", () =>
          HttpResponse.json(
            envelope({
              spec,
              spec_hash: "bound-hash",
              bound_parameter_ref: spec.parameter_ref,
            }),
          ),
        ),
      );
      render(<BuddyDesk />, { preloadedState: config });
      fireEvent.click(
        await screen.findByRole("button", { name: /Provider setup diagnosis/ }),
      );
      const preview = await screen.findByRole("button", {
        name: "Preview exact change",
      });
      fireEvent.click(preview);
      fireEvent.click(preview);
      expect(await screen.findByText("Saved durable proposal")).toBeTruthy();
      expect(previews).toEqual([{ spec }]);
      expect(
        screen.getAllByRole("article", { name: "Work detail" }),
      ).toHaveLength(1);
      expect(
        screen.getByRole("button", { name: "Grant and execute once" }),
      ).toBeDisabled();
    },
  );

  it("keeps unknown mutating adapter/kind pairs unavailable instead of attempting a generic resource action", async () => {
    setupActions(() => ({
      ...action,
      work_id: item.id,
      spec: {
        ...action.spec,
        adapter: "future_adapter",
        kind: "future_mutation",
        authority_class: "mutating",
        confirmation: "user_confirmation_required",
      },
    }));
    render(<BuddyDesk />, { preloadedState: config });
    expect(await openReport()).toBeDisabled();
    expect(screen.getByText(/not supported by this client/)).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "Preview exact change" }),
    ).toBeNull();
  });
});
