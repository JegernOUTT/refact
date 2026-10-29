import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
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
import { resetBuddyPersistentState } from "./ui";
import { BuddyDesk } from "./BuddyDesk";
import detailStyles from "./BuddyDeskDetail.module.css";
import type {
  ActionExecution,
  ArtifactRevision,
  RunOutcome,
  RunStatus,
  WorkItem,
  WorkItemState,
} from "./operatorTypes";
import {
  buddyOperatorApi,
  type DeskAction,
} from "../../services/refact/buddyOperator";
import { useGetBuddySnapshotQuery } from "../../services/refact/buddy";

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
HTMLElement.prototype.hasPointerCapture = () => false;
HTMLElement.prototype.releasePointerCapture = () => undefined;

async function chooseAttention(
  user: ReturnType<typeof render>["user"],
  label: string,
) {
  await user.click(screen.getByRole("combobox", { name: "Show reports" }));
  await user.click(await screen.findByRole("option", { name: label }));
}
const isUnreadLookup = (request: Request) =>
  new URL(request.url).searchParams.get("attention") === "unread" &&
  new URL(request.url).searchParams.get("limit") === "200";

function BuddySnapshotProbe() {
  useGetBuddySnapshotQuery(undefined);
  return null;
}

function readGuiSource(path: string): Promise<string> {
  return readFile(resolve(process.cwd(), "src", path), "utf8");
}

function readCssBlock(source: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`(^|\\n)\\s*${escaped}\\s*{`).exec(source);
  if (match?.index === undefined)
    throw new Error(`Missing CSS block for ${selector}`);
  const open = source.indexOf("{", match.index);
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    if (source[i] === "{") depth += 1;
    if (source[i] === "}") depth -= 1;
    if (depth === 0) return source.slice(open + 1, i);
  }
  throw new Error(`Unclosed CSS block for ${selector}`);
}
const envelope = <T,>(data: T, next_cursor: number | null = null) => ({
  schema_version: 1,
  project,
  data,
  next_cursor,
});

const batchEnvelope = (
  request: Request,
  actionsFor: (workId: string) => DeskAction[] = () => [],
  errorFor: (workId: string) => string | null = () => null,
) => {
  const ids = new URL(request.url).searchParams.get("ids")?.split(",") ?? [];
  return envelope(
    ids.map((work_id) => ({
      work_id,
      actions: actionsFor(work_id),
      error: errorFor(work_id),
    })),
  );
};

const workStateMatrix: Record<WorkItemState, { view: string; label: string }> =
  {
    needs_attention: { view: "Needs you", label: "Needs you" },
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
    resetBuddyPersistentState();
    server.use(
      http.get("*/v1/buddy/work/actions", ({ request }) =>
        HttpResponse.json(batchEnvelope(request)),
      ),
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
  const viewDefaultStates: WorkItemState[] = [
    "needs_attention",
    "working",
    "delivered",
    "watching",
  ];

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
      await screen.findByRole("button", {
        name: /Outcome needs_attention/,
        hidden: true,
      });
      fireEvent.click(screen.getByRole("radio", { name: view }));
      await waitFor(() => expect(requests).toContain(query));
      for (const state of states) {
        const expected = workStateMatrix[state];
        const rowName = new RegExp(`Outcome ${state}`);
        if (expected.view !== view) {
          expect(screen.queryByRole("button", { name: rowName })).toBeNull();
          continue;
        }
        const row = await screen.findByRole("button", {
          name: rowName,
          hidden: true,
        });
        const isViewDefault = viewDefaultStates.includes(state);
        if (isViewDefault) {
          expect(
            within(row).queryByText(expected.label, { exact: true }),
          ).toBeNull();
        } else {
          expect(
            within(row).getByText(expected.label, { exact: true }),
          ).toBeTruthy();
        }
        if (state === "cancelled") {
          expect(
            within(row).queryByText("Delivered", { exact: true }),
          ).toBeNull();
        }
        fireEvent.click(row);
        const detail = within(screen.getByRole("article"));
        expect(detail.getByText(expected.label, { exact: true })).toBeTruthy();
        expect(detail.getByText(item.subject.resource_id)).toBeTruthy();
        fireEvent.click(
          screen.getByRole("button", { name: "Back to reports" }),
        );
      }
      expect(screen.getByRole("radio", { name: view })).toBeChecked();
    },
  );

  it.each(Object.keys(runStatusMatrix) as RunStatus[])(
    "renders %s through recorded-check history with coverage, not verification",
    async (status) => {
      const expected = runStatusMatrix[status];
      const run: RunOutcome = {
        schema_version: 1,
        run_id: `buddy-check-${status}-550e8400-e29b-41d4-a716-446655440000`,
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
      fireEvent.click(screen.getByRole("radio", { name: workStatus.view }));
      fireEvent.click(
        await screen.findByRole("button", {
          name: /Provider setup diagnosis/,
          hidden: true,
        }),
      );
      await screen.findByText("The token has expired.");
      expect(historyRequests).toHaveLength(0);
      expect(outcomeRequests).toHaveLength(0);
      fireEvent.click(
        screen.getByRole("button", { name: /^Run history/, hidden: true }),
      );
      const details = screen.getByRole("region", { name: /^Run history/ });
      const row = await within(details).findByRole("button", {
        name: new RegExp(`^${expected.label}`),
        hidden: true,
      });
      expect(row).not.toHaveAccessibleName(new RegExp(run.run_id));
      expect(row).not.toHaveAccessibleName(/\d{4}-\d{2}-\d{2}T/);
      expect(row.querySelector("time")).toHaveAttribute(
        "dateTime",
        new Date(run.ended_at ?? run.started_at).toISOString(),
      );
      expect(historyRequests).toHaveLength(1);
      expect(outcomeRequests).toHaveLength(0);
      expect(details).not.toHaveTextContent(/verified/i);
      fireEvent.click(row);
      expect(row).toHaveAttribute("aria-current", "true");
      const evidence = within(
        screen.getByRole("region", { name: "Recorded run" }),
      );
      expect(await evidence.findByText(run.coverage)).toBeTruthy();
      const runSummary = evidence.getByRole("heading", {
        name: new RegExp(`^${expected.label} · `),
      });
      expect(runSummary).not.toHaveTextContent(/\d{4}-\d{2}-\d{2}T/);
      expect(runSummary).not.toHaveTextContent(/ – /);
      expect(
        evidence.getByRole("heading", { name: "What was checked" }),
      ).toBeTruthy();
      expect(evidence.getByText("No changes were recorded.")).toBeTruthy();
      expect(evidence.queryByText(new RegExp(run.run_id))).toBeNull();
      fireEvent.click(
        evidence.getByRole("button", { name: "Close recorded run" }),
      );
      expect(screen.queryByRole("region", { name: "Recorded run" })).toBeNull();
      expect(outcomeRequests).toHaveLength(1);
    },
  );

  it("read_and_snooze_copy_is_positive_and_still_unambiguous", async () => {
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
    const { user } = render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    const snooze = await screen.findByRole("button", {
      name: "Snooze for one hour",
    });
    await waitFor(() => expect(snooze).toBeEnabled());
    expect(
      screen.queryByText("Still open — a new revision ends the snooze."),
    ).toBeNull();
    fireEvent.click(snooze);
    fireEvent.click(snooze);
    const snoozeStatus = await screen.findByText(/Snoozed until/);
    expect(snoozeStatus).not.toHaveTextContent(/T\d{2}:\d{2}/);
    expect(snoozeStatus.querySelector("time")).toHaveAttribute(
      "dateTime",
      "2099-01-01T00:00:00.000Z",
    );
    expect(snoozeStatus).toHaveTextContent(/A new revision ends the snooze/);
    expect(writes).toHaveLength(1);
    expect(writes[0]).toMatchObject({
      project_id: project.project_id,
      canonical_root: project.canonical_root,
      minutes: 60,
      artifact_revision: 1,
    });
    expect(screen.getByRole("button", { name: "Mark as read" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "End snooze" }));
    expect(
      await screen.findByRole("button", { name: "Snooze for one hour" }),
    ).toBeTruthy();
    expect(writes[1].minutes).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Back to reports" }));
    await chooseAttention(user, "Unread reports");
    await waitFor(() => expect(filters).toContain("unread"));
  });

  it("defaults to active reports and exposes snoozed work only through archive filters", async () => {
    const filters: string[] = [];
    server.use(
      http.get("*/v1/buddy/work", ({ request }) => {
        const attention = new URL(request.url).searchParams.get("attention");
        filters.push(attention ?? "");
        return HttpResponse.json(
          envelope(attention === "unsnoozed" ? [] : [record(item)]),
        );
      }),
    );
    const { user } = render(<BuddyDesk />, { preloadedState: config });
    expect(
      await screen.findByText("Nothing needs you right now."),
    ).toBeTruthy();
    expect(filters[0]).toBe("unsnoozed");
    fireEvent.click(screen.getByRole("button", { name: "Browse reports" }));
    expect(
      screen.getByRole("combobox", { name: "Show reports" }),
    ).toHaveTextContent("Active reports");
    expect(
      screen.queryByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    ).toBeNull();
    await chooseAttention(user, "Snoozed reports");
    expect(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    ).toBeTruthy();
    await chooseAttention(user, "All reports, including snoozed");
    expect(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    ).toBeTruthy();
    expect(filters).toContain("snoozed");
    expect(filters).toContain("all");
    expect(
      screen.getByRole("combobox", { name: "Show reports" }),
    ).toHaveTextContent("All reports, including snoozed");
  });

  it("refetches the active list once after snoozing and keeps the report open", async () => {
    let listRequests = 0;
    let snapshotRequests = 0;
    let snoozed = false;
    const snoozeRecord = {
      schema_version: 1,
      project,
      work_id: item.id,
      artifact_id: artifact.artifact_id,
      artifact_revision: artifact.revision,
      until: "2099-01-01T00:00:00Z",
    };
    server.use(
      http.get("*/v1/buddy", () => {
        snapshotRequests += 1;
        return HttpResponse.json({});
      }),
      http.get("*/v1/buddy/work", ({ request }) => {
        if (isUnreadLookup(request)) return HttpResponse.json(envelope([]));
        listRequests += 1;
        const attention = new URL(request.url).searchParams.get("attention");
        const visible =
          attention === "snoozed" ? snoozed : attention === "all" || !snoozed;
        return HttpResponse.json(envelope(visible ? [record(item)] : []));
      }),
      http.get("*/v1/buddy/work/:id/snooze", () =>
        HttpResponse.json(envelope(snoozed ? snoozeRecord : null)),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.post("*/v1/buddy/work/:id/snooze", async ({ request }) => {
        const body = (await request.json()) as { minutes: number | null };
        snoozed = body.minutes !== null;
        return HttpResponse.json(envelope(snoozed ? snoozeRecord : null));
      }),
    );
    render(
      <>
        <BuddySnapshotProbe />
        <BuddyDesk />
      </>,
      { preloadedState: config },
    );
    await waitFor(() => expect(snapshotRequests).toBe(1));
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByRole("heading", { name: artifact.title });
    const snooze = await screen.findByRole("button", {
      name: "Snooze for one hour",
    });
    await waitFor(() => expect(snooze).toBeEnabled());
    fireEvent.click(snooze);
    expect(await screen.findByText(/Snoozed until/)).toBeTruthy();
    await waitFor(() => expect(listRequests).toBe(2));
    await waitFor(() =>
      expect(
        screen.queryByRole("button", {
          name: /Provider setup diagnosis/,
          hidden: true,
        }),
      ).toBeNull(),
    );
    expect(listRequests).toBe(2);
    expect(snapshotRequests).toBe(1);
    expect(
      screen.queryByText("That report is no longer in this view."),
    ).toBeNull();
    expect(screen.getByRole("article")).toBeTruthy();
    expect(screen.getByRole("heading", { name: artifact.title })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "End snooze" }));
    await waitFor(() => expect(listRequests).toBe(3));
    expect(listRequests).toBe(3);
    expect(snapshotRequests).toBe(1);
    expect(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Back to reports" }));
    await waitFor(() =>
      expect(document.activeElement?.matches("ul li button")).toBe(true),
    );
  });

  it("does not claim an unsnoozed report while its first snooze-status request is pending", async () => {
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
      http.get("*/v1/buddy/work/:id/snooze", async () => {
        await pending;
        return HttpResponse.json(envelope(null));
      }),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByRole("heading", { name: artifact.title });
    const snooze = within(screen.getByRole("region", { name: "Work snooze" }));
    expect(await snooze.findByRole("status")).toHaveTextContent(
      "Loading snooze status…",
    );
    expect(snooze.queryByText(/Snoozed until/)).toBeNull();
    expect(
      snooze.getByRole("button", { name: "Snooze for one hour" }),
    ).toBeDisabled();
    expect(
      snooze.getByRole("button", { name: "Snooze for one day" }),
    ).toBeDisabled();
    release?.();
    await waitFor(() =>
      expect(
        snooze.getByRole("button", { name: "Snooze for one hour" }),
      ).toBeEnabled(),
    );
    expect(snooze.queryByText(/Snoozed until/)).toBeNull();
  });

  it("loads a real API-shaped report without navigating to a worker chat", async () => {
    const headedArtifact = {
      ...artifact,
      body: "# diagnostic_assessment\n\n## Second level\n\n### Third level",
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", ({ request }) => {
        expect(new URL(request.url).searchParams.get("revision")).toBe("1");
        return HttpResponse.json(envelope(record(headedArtifact)));
      }),
    );
    const { store } = render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    fireEvent.click(row);
    expect(await screen.findByText("The token has expired.")).toBeTruthy();
    expect(
      screen.getByRole("heading", { level: 3, name: artifact.title }),
    ).toBeTruthy();
    const detail = screen.getByRole("article");
    const headings = [...detail.querySelectorAll("h1, h2, h3, h4, h5, h6")];
    expect(headings[0].tagName).toBe("H3");
    expect(headings[0]).toHaveTextContent(artifact.title);
    expect(headings.slice(1).every((heading) => heading.tagName === "H4")).toBe(
      true,
    );
    expect(headings.map((heading) => heading.textContent)).toEqual(
      expect.arrayContaining([
        "Diagnostic assessment",
        "Second level",
        "Third level",
      ]),
    );
    expect(detail).not.toHaveTextContent("diagnostic_assessment");
    expect(screen.getByText("Network access was not tested.")).toBeTruthy();
    expect(
      screen.getByText(/Suggested fixes are never applied automatically/),
    ).toBeTruthy();
    expect(store.getState().pages.at(-1)?.name).toBe("login page");
    fireEvent.click(screen.getByRole("button", { name: "Back to reports" }));
    await waitFor(() => expect(document.activeElement).toBe(row));
  });

  it("shows a multiline report title remainder once in the detail body", async () => {
    const multilineArtifact = {
      ...artifact,
      title: "Report headline\n\nExtra detail not in body",
      body: "Body content without the extra title detail.",
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(multilineArtifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    expect(
      await screen.findByRole("heading", {
        level: 3,
        name: "Report headline",
      }),
    ).toBeTruthy();
    const body = screen.getByText("Extra detail not in body");
    expect(body.closest(`.${detailStyles.disclosureBody}`)).toBeTruthy();
    const writeUp = screen.getByRole("region", { name: "Full write-up" });
    expect(
      within(writeUp).getAllByText("Extra detail not in body", { exact: true }),
    ).toHaveLength(1);
  });

  it("does not repeat a title remainder already present in the detail body", async () => {
    const multilineArtifact = {
      ...artifact,
      title: "Report headline\n\nExtra detail included",
      body: "The body already contains Extra detail included in this sentence.",
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(multilineArtifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    expect(
      await screen.findByRole("heading", {
        level: 3,
        name: "Report headline",
      }),
    ).toBeTruthy();
    const details = screen.getByRole("region", { name: "Full write-up" });
    expect(
      within(details).getAllByText(/Extra detail included/, { exact: false }),
    ).toHaveLength(1);
  });

  it("renders only a multiline title headline in the work row", async () => {
    const multilineItem = {
      ...item,
      title: "Headline\n\nBody text",
      severity: "high",
      scope: "src/config",
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(multilineItem)])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Headline/,
      hidden: true,
    });
    expect(within(row).getByTestId("buddy-desk-row-title")).toHaveTextContent(
      /^Headline$/,
    );
    expect(row).not.toHaveTextContent("Body text");
    expect(row).toHaveAccessibleName(/Headline/);
    expect(row).toHaveAccessibleName(/High/);
    expect(row).toHaveAccessibleName(/src\/config/);
    expect(row).not.toHaveAccessibleName(/Needs you/);
    expect(row).toHaveAccessibleName(/Changed /);
    expect(row).not.toHaveAccessibleName(/Body text/);
    expect(row).not.toHaveAttribute("aria-describedby");
    expect(row.querySelector("time")).toHaveAttribute(
      "dateTime",
      new Date(multilineItem.last_material_change_at).toISOString(),
    );
  });

  it("keeps the open report and shows its refreshed revision", async () => {
    let requests = 0;
    server.use(
      http.get("*/v1/buddy/work", ({ request }) => {
        if (isUnreadLookup(request)) return HttpResponse.json(envelope([]));
        requests += 1;
        const refreshed = requests > 1;
        return HttpResponse.json(
          envelope([
            record({
              ...item,
              title: refreshed ? "Refreshed diagnosis" : item.title,
              current_artifact_revision: refreshed ? 2 : 1,
            }),
          ]),
        );
      }),
      http.get("*/v1/buddy/artifacts/:id", ({ request }) => {
        const revision = new URL(request.url).searchParams.get("revision");
        return HttpResponse.json(
          envelope(
            record({
              ...artifact,
              revision: Number(revision ?? 1),
              title: revision === "2" ? "Refreshed assessment" : artifact.title,
            }),
          ),
        );
      }),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByRole("heading", { name: artifact.title });
    fireEvent.click(screen.getByRole("button", { name: "Refresh reports" }));
    expect(
      await screen.findByRole("heading", { name: "Refreshed assessment" }),
    ).toBeTruthy();
    expect(screen.getByRole("article")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /Refreshed diagnosis/, hidden: true }),
    ).toBeTruthy();
  });

  it("reconciles sequential refreshes and restores focus when an item disappears", async () => {
    let requests = 0;
    let release: (() => void) | undefined;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    server.use(
      http.get("*/v1/buddy/work", async ({ request }) => {
        if (isUnreadLookup(request)) return HttpResponse.json(envelope([]));
        requests += 1;
        if (requests === 2) await pending;
        return HttpResponse.json(
          envelope(
            requests === 3
              ? []
              : [
                  record({
                    ...item,
                    title:
                      requests === 2
                        ? "Still present after refresh"
                        : item.title,
                  }),
                ],
          ),
        );
      }),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByRole("heading", { name: artifact.title });
    const refresh = screen.getByRole("button", { name: "Refresh reports" });
    fireEvent.click(refresh);
    await waitFor(() => expect(requests).toBe(2));
    expect(screen.getByRole("article")).toBeTruthy();
    expect(
      screen.queryByText("That report is no longer in this view."),
    ).toBeNull();
    release?.();
    await screen.findByRole("button", {
      name: /Still present after refresh/,
      hidden: true,
    });
    await waitFor(() => expect(screen.queryByText("Loading work…")).toBeNull());
    fireEvent.click(refresh);
    expect(
      await screen.findByText("That report is no longer in this view."),
    ).toBeTruthy();
    expect(screen.queryByRole("article")).toBeNull();
    expect(
      screen.getAllByText("That report is no longer in this view."),
    ).toHaveLength(1);
    await waitFor(() => {
      const active = document.activeElement;
      expect(
        Boolean(active?.matches("ul li button")) || active === refresh,
      ).toBe(true);
    });
    expect(document.activeElement).not.toBe(document.body);
  });

  it("waits for one in-flight request after a rapid second refresh click", async () => {
    let requests = 0;
    let release: (() => void) | undefined;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    server.use(
      http.get("*/v1/buddy/work", async ({ request }) => {
        if (isUnreadLookup(request)) return HttpResponse.json(envelope([]));
        requests += 1;
        if (requests > 1) await pending;
        return HttpResponse.json(envelope(requests > 1 ? [] : [record(item)]));
      }),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByRole("heading", { name: artifact.title });
    const refresh = screen.getByRole("button", { name: "Refresh reports" });
    fireEvent.click(refresh);
    fireEvent.click(refresh);
    await waitFor(() => expect(requests).toBe(2));
    expect(screen.getByRole("article")).toBeTruthy();
    expect(
      screen.queryByText("That report is no longer in this view."),
    ).toBeNull();
    release?.();
    expect(
      await screen.findByText("That report is no longer in this view."),
    ).toBeTruthy();
    expect(requests).toBe(2);
    expect(
      screen.getAllByText("That report is no longer in this view."),
    ).toHaveLength(1);
    expect(screen.queryByRole("article")).toBeNull();
    expect(document.activeElement).not.toBe(document.body);
  });

  it("keeps the selection on refresh error and reconciles after Retry work succeeds", async () => {
    let requests = 0;
    let release: (() => void) | undefined;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    server.use(
      http.get("*/v1/buddy/work", async ({ request }) => {
        if (isUnreadLookup(request)) return HttpResponse.json(envelope([]));
        requests += 1;
        if (requests === 2) {
          await pending;
          return HttpResponse.json({ detail: "unavailable" }, { status: 503 });
        }
        return HttpResponse.json(
          envelope(requests === 3 ? [] : [record(item)]),
        );
      }),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByRole("heading", { name: artifact.title });
    fireEvent.click(screen.getByRole("button", { name: "Refresh reports" }));
    await waitFor(() => expect(requests).toBe(2));
    expect(screen.getByTestId("buddy-desk-layout")).toHaveAttribute(
      "data-detail-open",
      "true",
    );
    release?.();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't load reports.",
    );
    expect(
      screen.queryByText("That report is no longer in this view."),
    ).toBeNull();
    expect(screen.getByTestId("buddy-desk-layout")).toHaveAttribute(
      "data-detail-open",
      "true",
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry reports" }));
    expect(
      await screen.findByText("That report is no longer in this view."),
    ).toBeTruthy();
    expect(requests).toBe(3);
    expect(screen.queryByRole("article")).toBeNull();
  });

  it("leaves a deep-linked item open when the refreshed list omits it", async () => {
    let requests = 0;

    server.use(
      http.get("*/v1/buddy/work", ({ request }) => {
        if (isUnreadLookup(request)) return HttpResponse.json(envelope([]));
        requests += 1;
        return HttpResponse.json(
          envelope(requests === 1 ? [record(item)] : []),
        );
      }),
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
    await screen.findByRole("heading", { name: artifact.title });
    fireEvent.click(screen.getByRole("button", { name: "Refresh reports" }));
    expect(
      await screen.findByText("Nothing needs you right now."),
    ).toBeTruthy();
    await waitFor(() => expect(requests).toBe(2));
    expect(screen.getByRole("article")).toBeTruthy();
    expect(
      screen.queryByText("That report is no longer in this view."),
    ).toBeNull();
  });

  it("hides workflow machine identifiers from the detail header", async () => {
    const workflowItem: WorkItem = {
      ...item,
      subject: {
        ...item.subject,
        resource_kind: "workflow",
        resource_id: "buddy_security_whisperer",
      },
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(workflowItem)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    const detail = await screen.findByRole("article");
    expect(detail).not.toHaveTextContent("buddy_security_whisperer");
    expect(within(detail).getByText("Needs you", { exact: true })).toBeTruthy();
  });

  it("hides UUID-shaped subject identifiers from the detail header", async () => {
    const uuidItem: WorkItem = {
      ...item,
      subject: {
        ...item.subject,
        resource_id: "3f155984-a130-4c90-a1fd-ff2908bc4921",
      },
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(uuidItem)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    const detail = await screen.findByRole("article");
    expect(detail).not.toHaveTextContent(uuidItem.subject.resource_id);
    expect(within(detail).getByText("Needs you", { exact: true })).toBeTruthy();
  });

  it("renders evidence timestamps relatively with an absolute tooltip", async () => {
    const observedAt = "2026-09-05T00:00:00Z";
    const evidenceArtifact = {
      ...artifact,
      evidence: [
        {
          evidence_id: "evidence-1",
          source: {
            source_kind: "file",
            source_id: "Cargo.toml",
            source_version: "1",
          },
          locator: "Cargo.toml:1",
          observed_at: observedAt,
          digest: null,
        },
        {
          evidence_id: "evidence-invalid-date",
          source: {
            source_kind: "file",
            source_id: "Cargo.lock",
            source_version: "1",
          },
          locator: "Cargo.lock:1",
          observed_at: "not-a-valid-date",
          digest: null,
        },
      ],
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(evidenceArtifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: /^Evidence/ }));
    const observed = await screen.findByText(
      /Observed (\d+d ago|[A-Z][a-z]{2} \d+)/,
    );
    expect(observed).toHaveAttribute(
      "dateTime",
      new Date(observedAt).toISOString(),
    );
    expect(observed).not.toHaveTextContent(observedAt);
    expect(screen.queryByText("Observed not-a-valid-date")).toBeNull();
  });

  it("collapses to one honest line instead of empty desk chrome", async () => {
    server.use(
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
    );
    render(<BuddyDesk />, { preloadedState: config });
    expect(
      await screen.findByText("Nothing needs you right now."),
    ).toBeTruthy();
    const desk = screen.getByRole("region", { name: "Reports" });
    expect(within(desk).getByRole("heading", { name: "Reports" })).toBeTruthy();
    expect(within(desk).queryByRole("searchbox")).toBeNull();
    expect(
      within(desk).queryByRole("combobox", { name: "Show reports" }),
    ).toBeNull();
    expect(within(desk).queryByRole("radio", { name: "Needs you" })).toBeNull();
    expect(within(desk).queryByText(/on the left/)).toBeNull();
    fireEvent.click(
      within(desk).getByRole("button", { name: "Browse reports" }),
    );
    expect(
      await screen.findByRole("radio", { name: "Needs you" }),
    ).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Reports" })).toBeTruthy();
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
    expect(screen.getByRole("heading", { name: "Reports" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Browse reports" })).toBeNull();
  });

  it("reports a failed request rather than collapsing as if there were no work", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json({ detail: "unavailable" }, { status: 503 }),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't load reports.",
    );
    expect(screen.queryByText("Nothing needs you right now.")).toBeNull();
    expect(screen.getByRole("heading", { name: "Reports" })).toBeTruthy();
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
      hidden: true,
    });
    expect(row).not.toHaveTextContent(digest);
    expect(row).not.toHaveTextContent("Needs you");
    fireEvent.click(row);
    const detail = within(screen.getByRole("article"));
    expect(detail.getByText("Needs you")).toBeTruthy();
    expect(screen.queryByText(new RegExp(digest))).toBeNull();
  });

  it("rows_with_distinct_scopes_render_distinct_subtitles", async () => {
    const triaged = [
      ["Mode prompts are overlapping", "chat/modes", "high"],
      ["AGENTS.md missing or outdated", "AGENTS.md", "normal"],
      ["Security Whisperer source check", "src/security", "critical"],
      ["Setup Coach run outcome", "project setup", "low"],
      ["Dependency Radar source check", "Cargo.lock", "normal"],
    ] as const;
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope(
            triaged.map(([title, scope, severity], index) =>
              record({
                ...item,
                id: `triage-${index}`,
                title,
                severity,
                scope,
                problem_summary: null,
                current_artifact_id: null,
                current_artifact_revision: null,
              }),
            ),
          ),
        ),
      ),
      http.get("*/v1/buddy/work/actions", ({ request }) =>
        HttpResponse.json(batchEnvelope(request)),
      ),
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const rows = await Promise.all(
      triaged.map(([title]) =>
        screen.findByRole("button", { name: new RegExp(title), hidden: true }),
      ),
    );
    const subtitles = rows.map((row) => row.textContent);
    expect(new Set(subtitles).size).toBe(triaged.length);
    for (const [title, scope] of triaged) {
      if (title.includes(scope))
        expect(screen.queryByText(scope, { exact: true })).toBeNull();
      else expect(screen.getByText(scope, { exact: true })).toBeTruthy();
    }
  });

  it("absent_triage_fields_render_a_clean_row", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope([
            record({
              ...item,
              severity: null,
              problem_summary: null,
              scope: null,
            }),
          ]),
        ),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    expect(within(row).queryByTestId("buddy-desk-row-severity")).toBeNull();
    expect(row).toHaveTextContent(/^Provider setup diagnosis/);
    expect(
      within(row).getAllByText("Provider setup diagnosis", { exact: false }),
    ).toHaveLength(1);
    expect(row).not.toHaveTextContent("·");
  });

  it("unknown_severity_renders_no_indicator", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record({ ...item, severity: "unknown" })])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    expect(within(row).queryByTestId("buddy-desk-row-severity")).toBeNull();
  });

  it("severity_is_not_colour_only", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope([record({ ...item, severity: "critical" })]),
        ),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    const severity = within(row).getByTestId("buddy-desk-row-severity");
    expect(severity).toHaveTextContent("Critical");
    expect(severity).toHaveAttribute("data-tone", "danger");
  });

  it("problem_summary_equal_to_title_is_not_repeated", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope([
            record({ ...item, problem_summary: "Provider setup diagnosis" }),
          ]),
        ),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    expect(row).toHaveTextContent(/^Provider setup diagnosis/);
    expect(
      within(row).getAllByText("Provider setup diagnosis", { exact: false }),
    ).toHaveLength(1);
  });

  it("desk_hint_is_only_displayed_in_the_split_container_layout", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record({ ...item, state: "delivered" })])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(await screen.findByRole("radio", { name: "Delivered" }));
    expect(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    ).toBeTruthy();
    expect(screen.queryByRole("article")).toBeNull();
    expect(screen.getByTestId("buddy-desk-layout")).toHaveAttribute(
      "data-detail-open",
      "false",
    );
    expect(screen.getByText("Pick a report to read it here")).toBeTruthy();
    const css = await readGuiSource("features/Buddy/BuddyDesk.module.css");
    expect(readCssBlock(css, ".detailHint")).toContain("display: none");
    const splitCss = readCssBlock(css, "@container (min-width: 880px)");
    expect(readCssBlock(splitCss, ".detailHint")).toContain("display: block");
    expect(readCssBlock(splitCss, ".layout")).toContain(
      "grid-template-columns",
    );
    fireEvent.click(screen.getByRole("radio", { name: "Watching" }));
    expect(
      await screen.findByText("Nothing is being watched right now."),
    ).toBeTruthy();
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
    expect(screen.getByRole("button", { name: "Retry reports" })).toBeTruthy();
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
      screen.queryByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
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
    expect(await screen.findByText(/No other report was opened/)).toBeTruthy();
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
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByText("The token has expired.");
    expect(reads).toBe(0);
    fireEvent.click(screen.getByRole("button", { name: "Earlier revisions" }));
    const revision = await screen.findByRole("button", {
      name: "Revision 1 · Earlier assessment",
    });
    expect(revision).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(revision);
    expect(
      screen.getByText(
        "Earlier revision. Actions apply to the current revision only.",
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
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    const mark = await screen.findByRole("button", {
      name: "Mark as read",
    });
    await waitFor(() => expect(mark).toBeEnabled());
    expect(calls).toBe(0);
    fireEvent.click(mark);
    fireEvent.click(mark);
    await waitFor(() => expect(calls).toBe(1));
    expect(screen.queryByText("This revision is marked read.")).toBeNull();
    release?.();
    expect(
      await screen.findByText("This revision is marked read."),
    ).toBeTruthy();
    expect(
      within(
        screen.getByRole("region", { name: "Report reading status" }),
      ).getByText("Read", { exact: true }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Back to reports" }));
    fireEvent.click(
      screen.getByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    expect(
      await screen.findByText("This revision is marked read."),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    ).not.toHaveTextContent("Delivered");
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
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    const mark = await screen.findByRole("button", {
      name: "Mark as read",
    });
    await waitFor(() => expect(mark).toBeEnabled());
    fireEvent.click(mark);
    expect(
      await screen.findByText(/Couldn't save reading status/),
    ).toBeTruthy();
    expect(screen.queryByText("This revision is marked read.")).toBeNull();
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
          Boolean(cursor) || search === "Second"
            ? envelope([
                record({ ...item, id: "work-2", title: "Second diagnosis" }),
              ])
            : envelope([record(item)], 1),
        );
      }),
      http.get("*/v1/buddy/work/actions", ({ request }) =>
        HttpResponse.json(batchEnvelope(request)),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: "Load more reports" }),
    );
    expect(
      await screen.findByRole("button", {
        name: /Second diagnosis/,
        hidden: true,
      }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    ).toBeTruthy();
    fireEvent.change(screen.getByRole("searchbox"), {
      target: { value: "Second" },
    });
    expect(
      screen.queryByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    ).toBeNull();
    expect(
      await screen.findByRole("button", {
        name: /Second diagnosis/,
        hidden: true,
      }),
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
  const dismissAction: DeskAction = {
    ...action,
    spec: {
      ...action.spec,
      action_id: "dismiss_not_a_problem",
      kind: "dismiss_not_a_problem",
      adapter: "operator.work_lifecycle",
      resource: {
        resource_kind: "work_item",
        resource_id: item.id,
        version: "digest",
      },
      authority_class: "mutating",
      capabilities: {
        can_preview: false,
        can_execute: true,
        can_verify: true,
        can_recover: false,
        can_cancel: false,
      },
    },
    spec_hash: "dismiss-hash",
    label: "Dismiss — not a problem",
    description: "Moves this item out of Needs you.",
  };
  const resourceAction: DeskAction = {
    ...action,
    work_id: item.id,
    spec: {
      ...action.spec,
      action_id: "apply_saved_setting",
      kind: "markdown_project_apply",
      adapter: "extensions",
      resource: {
        resource_kind: "settings",
        resource_id: "provider-config",
        version: "digest",
      },
      authority_class: "mutating",
      confirmation: "user_confirmation_required",
    },
    spec_hash: "resource-hash",
    label: "Apply saved setting",
    description: "Applies the saved setting proposal.",
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
  const dismissalReceipt = (key: string): ActionExecution => ({
    ...receipt(key),
    action_id: dismissAction.spec.action_id,
    spec_hash: dismissAction.spec_hash,
    requester: {
      source_kind: "external",
      source_id: "buddy-desk:dismiss",
      source_version: "1",
    },
    status: "completed",
    verification_evidence: ["Work item dismissed by user request."],
  });
  function setupActions(getAction: () => DeskAction = () => action) {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.get("*/v1/buddy/work/actions", ({ request }) =>
        HttpResponse.json(
          batchEnvelope(request, (id) => (id === item.id ? [getAction()] : [])),
        ),
      ),
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([getAction()])),
      ),
    );
  }
  async function openReport() {
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
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
    expect(screen.queryByText("Saved evidence check: Partial")).toBeNull();
    release?.();
    expect(
      await screen.findByText("Saved evidence check: Partial"),
    ).toBeTruthy();
    expect(
      screen.getByText("Findings are not independently verified."),
    ).toBeTruthy();
    expect(store.getState().pages.at(-1)?.name).toBe("login page");
    fireEvent.click(screen.getByRole("button", { name: "Back to reports" }));
    await openReport();
    expect(
      await screen.findByText("Saved evidence check: Partial"),
    ).toBeTruthy();
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
    expect(screen.queryByText("Saved evidence check: Partial")).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: "Check saved evidence links" }),
    );
    expect(
      await screen.findByText("Saved evidence check: Partial"),
    ).toBeTruthy();
    expect(keys).toHaveLength(2);
    expect(keys[0]).toBe(keys[1]);
  });

  it.each([
    {
      name: "next row",
      rows: [
        { ...item, id: "work-before", title: "Earlier finding" },
        { ...item, title: "Dismissed finding\nextra details" },
        { ...item, id: "work-after", title: "Following finding" },
      ],
      focusTitle: "Following finding",
    },
    {
      name: "previous row when the dismissed row was last",
      rows: [
        { ...item, id: "work-before", title: "Earlier finding" },
        { ...item, title: "Dismissed finding\nextra details" },
      ],
      focusTitle: "Earlier finding",
    },
    {
      name: "Browse saved work when the dismissed row was the only row",
      rows: [{ ...item, title: "Dismissed finding\nextra details" }],
      focusTitle: null,
    },
  ])(
    "announces dismissal and focuses the $name",
    async ({ rows, focusTitle }) => {
      let currentRows = rows;
      let executions = 0;
      let releaseExecution: (() => void) | undefined;
      const pendingExecution = new Promise<void>((resolve) => {
        releaseExecution = resolve;
      });
      let workRequests = 0;
      server.use(
        http.get("*/v1/buddy/work", ({ request }) => {
          if (isUnreadLookup(request)) return HttpResponse.json(envelope([]));
          workRequests += 1;
          return HttpResponse.json(envelope(currentRows.map(record)));
        }),
        http.get("*/v1/buddy/artifacts/:id", () =>
          HttpResponse.json(envelope(record(artifact))),
        ),
        http.get("*/v1/buddy/work/actions", ({ request }) =>
          HttpResponse.json(
            batchEnvelope(request, (id) =>
              id === item.id ? [dismissAction] : [],
            ),
          ),
        ),
        http.get("*/v1/buddy/work/:id/actions", ({ params }) =>
          HttpResponse.json(
            envelope(params.id === item.id ? [dismissAction] : []),
          ),
        ),
        http.post(
          "*/v1/buddy/work/:id/actions/:action/execute",
          async ({ params, request }) => {
            expect(params.action).toBe("dismiss_not_a_problem");
            const body = (await request.json()) as { idempotency_key: string };
            executions += 1;
            currentRows = currentRows.filter((row) => row.id !== item.id);
            await pendingExecution;
            return HttpResponse.json(
              envelope(record(dismissalReceipt(body.idempotency_key))),
            );
          },
        ),
      );
      render(<BuddyDesk />, { preloadedState: config });
      const dismissed = await screen.findByRole("button", {
        name: /Dismissed finding/,
        hidden: true,
      });
      fireEvent.click(dismissed);
      fireEvent.click(
        await screen.findByRole("button", { name: dismissAction.label }),
      );
      await waitFor(() => expect(executions).toBe(1));
      const requestsBeforeRefresh = workRequests;
      expect(requestsBeforeRefresh).toBe(1);
      fireEvent.click(screen.getByRole("button", { name: "Refresh reports" }));
      await waitFor(() => expect(workRequests).toBe(requestsBeforeRefresh + 1));
      await waitFor(() =>
        expect(
          screen.queryByRole("button", {
            name: /Dismissed finding/,
            hidden: true,
          }),
        ).toBeNull(),
      );
      expect(screen.getByRole("article")).toBeTruthy();
      expect(
        screen.queryByText("That report is no longer in this view."),
      ).toBeNull();
      releaseExecution?.();
      const message =
        'Dismissed "Dismissed finding". You can find it under Delivered.';
      expect(await screen.findByText(message)).toBeTruthy();
      await waitFor(() =>
        expect(
          screen.queryByRole("button", {
            name: /Dismissed finding/,
            hidden: true,
          }),
        ).toBeNull(),
      );
      expect(screen.getAllByText(message)).toHaveLength(1);
      expect(
        screen.queryByText("That report is no longer in this view."),
      ).toBeNull();
      if (focusTitle) {
        const focusedRow = await screen.findByRole("button", {
          name: new RegExp(focusTitle),
          hidden: true,
        });
        await waitFor(() => expect(document.activeElement).toBe(focusedRow));
      } else {
        const browse = screen.getByRole("button", {
          name: "Browse reports",
        });
        await waitFor(() => expect(document.activeElement).toBe(browse));
      }
      expect(executions).toBe(1);
      await waitFor(() => expect(workRequests).toBe(requestsBeforeRefresh + 2));
    },
  );

  it("focuses Refresh work when dismissal removes the only row from an expanded Desk", async () => {
    let currentRows: WorkItem[] = [];
    let workRequests = 0;
    let executions = 0;
    let releaseExecution: (() => void) | undefined;
    const pendingExecution = new Promise<void>((resolve) => {
      releaseExecution = resolve;
    });
    server.use(
      http.get("*/v1/buddy/work", ({ request }) => {
        if (isUnreadLookup(request)) return HttpResponse.json(envelope([]));
        workRequests += 1;
        return HttpResponse.json(envelope(currentRows.map(record)));
      }),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([dismissAction])),
      ),
      http.post(
        "*/v1/buddy/work/:id/actions/:action/execute",
        async ({ request }) => {
          const body = (await request.json()) as { idempotency_key: string };
          executions += 1;
          currentRows = [];
          await pendingExecution;
          return HttpResponse.json(
            envelope(record(dismissalReceipt(body.idempotency_key))),
          );
        },
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: "Browse reports" }),
    );
    currentRows = [item];
    fireEvent.click(screen.getByRole("button", { name: "Refresh reports" }));
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    fireEvent.click(row);
    fireEvent.click(
      await screen.findByRole("button", { name: dismissAction.label }),
    );
    await waitFor(() => expect(executions).toBe(1));
    const requestsBeforeRefresh = workRequests;
    expect(requestsBeforeRefresh).toBe(2);
    fireEvent.click(screen.getByRole("button", { name: "Refresh reports" }));
    await waitFor(() => expect(workRequests).toBe(requestsBeforeRefresh + 1));
    await waitFor(() =>
      expect(
        screen.queryByRole("button", {
          name: /Provider setup diagnosis/,
          hidden: true,
        }),
      ).toBeNull(),
    );
    expect(screen.getByRole("article")).toBeTruthy();
    expect(
      screen.queryByText("That report is no longer in this view."),
    ).toBeNull();
    releaseExecution?.();
    expect(
      await screen.findByText(
        'Dismissed "Provider setup diagnosis". You can find it under Delivered.',
      ),
    ).toBeTruthy();
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("button", { name: "Refresh reports" }),
      ),
    );
    expect(
      screen.queryByText("That report is no longer in this view."),
    ).toBeNull();
    await waitFor(() => expect(workRequests).toBe(requestsBeforeRefresh + 2));
  });

  it("does not refetch the Desk list when dismissal starts", async () => {
    setupActions(() => dismissAction);
    let workRequests = 0;
    let executions = 0;
    let releaseExecution: (() => void) | undefined;
    const pendingExecution = new Promise<void>((resolve) => {
      releaseExecution = resolve;
    });
    server.use(
      http.get("*/v1/buddy/work", ({ request }) => {
        if (isUnreadLookup(request)) return HttpResponse.json(envelope([]));
        workRequests += 1;
        return HttpResponse.json(envelope([record(item)]));
      }),
      http.post(
        "*/v1/buddy/work/:id/actions/:action/execute",
        async ({ request }) => {
          const body = (await request.json()) as { idempotency_key: string };
          executions += 1;
          await pendingExecution;
          return HttpResponse.json(
            envelope(record(dismissalReceipt(body.idempotency_key))),
          );
        },
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    fireEvent.click(
      await screen.findByRole("button", { name: dismissAction.label }),
    );
    await waitFor(() => expect(executions).toBe(1));
    expect(workRequests).toBe(1);
    releaseExecution?.();
    expect(
      await screen.findByText(
        'Dismissed "Provider setup diagnosis". You can find it under Delivered.',
      ),
    ).toBeTruthy();
    await waitFor(() => expect(workRequests).toBe(2));
  });

  it("keeps a failed dismissal open without an announcement", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([dismissAction])),
      ),
      http.post("*/v1/buddy/work/:id/actions/:action/execute", () =>
        HttpResponse.json({ detail: "unavailable" }, { status: 503 }),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    const dismiss = await screen.findByRole("button", {
      name: dismissAction.label,
    });
    fireEvent.click(dismiss);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "No acknowledged result was received.",
    );
    expect(screen.getByRole("article")).toBeTruthy();
    expect(
      screen.queryByText(
        /^Dismissed ".*"\. You can find it under Delivered\.$/,
      ),
    ).toBeNull();
    expect(
      screen.getByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    ).toBeTruthy();
  });

  it("does not announce or close dismissal without a completed execution", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([dismissAction])),
      ),
      http.post(
        "*/v1/buddy/work/:id/actions/:action/execute",
        async ({ request }) => {
          const body = (await request.json()) as { idempotency_key: string };
          return HttpResponse.json(
            envelope(
              record({
                ...dismissalReceipt(body.idempotency_key),
                status: "running",
              }),
            ),
          );
        },
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    fireEvent.click(
      await screen.findByRole("button", { name: dismissAction.label }),
    );
    expect(
      await screen.findByText("Dismissed from Needs you: Running"),
    ).toBeTruthy();
    expect(
      screen.queryByText(
        /^Dismissed "Provider setup diagnosis"\. You can find it under Delivered\.$/,
      ),
    ).toBeNull();
    expect(screen.getByRole("article")).toBeTruthy();
    expect(
      screen.getByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    ).toBeTruthy();
    expect(
      screen.queryByText("That report is no longer in this view."),
    ).toBeNull();
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
    fireEvent.click(screen.getByRole("button", { name: "Refresh actions" }));
    expect(
      await screen.findByText(
        /No actions are available for this report revision/,
      ),
    ).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "Check saved evidence links" }),
    ).toBeNull();
  });

  it("dismiss_remains_the_last_action", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([resourceAction, action, dismissAction])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByRole("button", { name: dismissAction.label });
    const labels = [...document.querySelectorAll("section[aria-label]")]
      .map((section) => section.getAttribute("aria-label"))
      .filter((label) =>
        [resourceAction.label, action.label, dismissAction.label].includes(
          label ?? "",
        ),
      );
    expect(labels).toEqual([
      resourceAction.label,
      action.label,
      dismissAction.label,
    ]);
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
        await screen.findByRole("button", {
          name: /Provider setup diagnosis/,
          hidden: true,
        }),
      );
      const preview = await screen.findByRole("button", {
        name: "Preview exact change",
      });
      fireEvent.click(preview);
      fireEvent.click(preview);
      expect(await screen.findByText("Saved durable proposal")).toBeTruthy();
      expect(previews).toEqual([{ spec }]);
      expect(screen.getAllByRole("article")).toHaveLength(1);
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

  it("gives every row its registered action verb or an honest no-action phrase", async () => {
    const withoutAction: WorkItem = {
      ...item,
      id: "work-2",
      title: "Second diagnosis",
    };
    const withoutResult: WorkItem = {
      ...item,
      id: "work-3",
      title: "Third diagnosis",
      current_artifact_id: null,
      current_artifact_revision: null,
    };
    const batchIds: string[][] = [];
    const actionRequests: string[] = [];
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope([
            record(item),
            record(withoutAction),
            record(withoutResult),
          ]),
        ),
      ),
      http.get("*/v1/buddy/work/actions", ({ request }) => {
        batchIds.push(
          new URL(request.url).searchParams.get("ids")?.split(",") ?? [],
        );
        return HttpResponse.json(
          batchEnvelope(request, (id) => (id === item.id ? [action] : [])),
        );
      }),
      http.get("*/v1/buddy/work/:id/actions", ({ params }) => {
        actionRequests.push(String(params.id));
        return HttpResponse.json(
          envelope(params.id === item.id ? [action] : []),
        );
      }),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    expect(
      await within(row).findByText("Check saved evidence links", {
        exact: true,
      }),
    ).toBeTruthy();
    const plain = screen.getByRole("button", {
      name: /Second diagnosis/,
      hidden: true,
    });
    expect(
      await within(plain).findByText("No action to take", { exact: true }),
    ).toBeTruthy();
    const unfinished = screen.getByRole("button", {
      name: /Third diagnosis/,
      hidden: true,
    });
    expect(
      within(unfinished).getByText("No result yet", { exact: true }),
    ).toBeTruthy();
    expect(batchIds).toHaveLength(1);
    expect(batchIds[0]).not.toContain(withoutResult.id);
    expect(batchIds[0]).toContain(item.id);
    expect(batchIds[0]).toContain(withoutAction.id);
    expect(actionRequests).toHaveLength(0);
  });

  it("loads row actions once and stays quiet while the Desk is idle", async () => {
    const rows = Array.from({ length: 8 }, (_, index) => ({
      ...item,
      id: `idle-work-${index}`,
      title: `Idle report ${index}`,
      current_artifact_id: `idle-artifact-${index}`,
    }));
    const requests: string[][] = [];
    const perRowRequests: string[] = [];
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope(rows.map((row) => record(row)))),
      ),
      http.get("*/v1/buddy/work/actions", ({ request }) => {
        requests.push(
          new URL(request.url).searchParams.get("ids")?.split(",") ?? [],
        );
        return HttpResponse.json(batchEnvelope(request));
      }),
      http.get("*/v1/buddy/work/:id/actions", ({ params }) => {
        perRowRequests.push(String(params.id));
        return HttpResponse.json(envelope([]));
      }),
    );
    const view = render(<BuddyDesk />, { preloadedState: config });
    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual(rows.map((row) => row.id));
    expect(perRowRequests).toHaveLength(0);

    view.unmount();
    const remounted = render(<BuddyDesk />, { store: view.store });
    await waitFor(() => expect(requests).toHaveLength(1));

    expect(requests).toHaveLength(1);
    expect(perRowRequests).toHaveLength(0);
    remounted.unmount();
  });

  describe("Desk row verbs through the batch endpoint", () => {
    const firstAction = (id: string, revision = 1): DeskAction => ({
      ...action,
      spec: {
        ...action.spec,
        resource: { ...action.spec.resource, resource_id: id },
      },
      artifact_id: `artifact-${id}`,
      artifact_revision: revision,
    });
    const rows = [
      {
        ...item,
        id: "no-artifact",
        title: "No artifact report",
        current_artifact_id: null,
        current_artifact_revision: null,
      },
      {
        ...item,
        id: "entry-error",
        title: "Entry error report",
        current_artifact_id: "artifact-entry-error",
      },
      {
        ...item,
        id: "matching-action",
        title: "Matching action report",
        current_artifact_id: "artifact-matching-action",
      },
      {
        ...item,
        id: "old-action",
        title: "Old action report",
        current_artifact_id: "artifact-old-action",
      },
    ];
    const actionsFor = (id: string): DeskAction[] => {
      if (id === "matching-action") return [firstAction(id)];
      if (id === "old-action") return [firstAction(id, 2)];
      return [];
    };
    async function renderRows() {
      render(<BuddyDesk />, { preloadedState: config });
      for (const row of rows)
        await screen.findByRole("button", {
          name: new RegExp(row.title),
          hidden: true,
        });
    }

    it("loads one page and renders the matching verb states", async () => {
      const batchIds: string[][] = [];
      const perRowRequests: string[] = [];
      server.use(
        http.get("*/v1/buddy/work", () =>
          HttpResponse.json(envelope(rows.map(record))),
        ),
        http.get("*/v1/buddy/work/actions", ({ request }) => {
          batchIds.push(
            new URL(request.url).searchParams.get("ids")?.split(",") ?? [],
          );
          return HttpResponse.json(
            batchEnvelope(request, actionsFor, (id) =>
              id === "entry-error" ? "unavailable" : null,
            ),
          );
        }),
        http.get("*/v1/buddy/work/:id/actions", ({ params }) => {
          perRowRequests.push(String(params.id));
          return HttpResponse.json(envelope([]));
        }),
      );
      await renderRows();
      expect({
        batches: batchIds.length,
        perRow: perRowRequests.length,
      }).toEqual({ batches: 1, perRow: 0 });
      for (const [id, verb] of [
        [rows[0].id, "No result yet"],
        [rows[1].id, "Action status unavailable"],
        [rows[2].id, action.label],
        [rows[3].id, "No action to take"],
      ]) {
        const source = rows.find((candidate) => candidate.id === id);
        if (!source) throw new Error(`Missing test row ${id}`);
        const row = screen.getByRole("button", {
          name: new RegExp(source.title),
          hidden: true,
        });
        expect(
          await within(row).findByText(verb, { exact: true }),
        ).toBeTruthy();
      }
    });

    it.each([404, 405])(
      "falls back to per-row queries for HTTP %s",
      async (status) => {
        const perRowRequests: string[] = [];
        server.use(
          http.get("*/v1/buddy/work", () =>
            HttpResponse.json(envelope(rows.map(record))),
          ),
          http.get("*/v1/buddy/work/actions", () =>
            HttpResponse.json({ detail: "not supported" }, { status }),
          ),
          http.get("*/v1/buddy/work/:id/actions", ({ params }) => {
            const id = String(params.id);
            perRowRequests.push(id);
            return HttpResponse.json(
              envelope(id === "matching-action" ? [firstAction(id)] : []),
            );
          }),
        );
        await renderRows();
        expect(perRowRequests.sort()).toEqual(
          rows
            .filter((row) => row.current_artifact_id)
            .map((row) => row.id)
            .sort(),
        );
        expect(
          within(
            screen.getByRole("button", {
              name: /No artifact report/,
              hidden: true,
            }),
          ).getByText("No result yet", { exact: true }),
        ).toBeTruthy();
        expect(
          await within(
            screen.getByRole("button", {
              name: /Entry error report/,
              hidden: true,
            }),
          ).findByText("No action to take", { exact: true }),
        ).toBeTruthy();
        expect(
          await within(
            screen.getByRole("button", {
              name: /Matching action report/,
              hidden: true,
            }),
          ).findByText(action.label, { exact: true }),
        ).toBeTruthy();
        expect(
          await within(
            screen.getByRole("button", {
              name: /Old action report/,
              hidden: true,
            }),
          ).findByText("No action to take", { exact: true }),
        ).toBeTruthy();
      },
    );

    it.each([503, "network"])(
      "does not fall back after batch failure %s",
      async (failure) => {
        const perRowRequests: string[] = [];
        server.use(
          http.get("*/v1/buddy/work", () =>
            HttpResponse.json(envelope(rows.map(record))),
          ),
          http.get("*/v1/buddy/work/actions", () =>
            failure === "network"
              ? HttpResponse.error()
              : HttpResponse.json(
                  { detail: "unavailable" },
                  { status: typeof failure === "number" ? failure : 503 },
                ),
          ),
          http.get("*/v1/buddy/work/:id/actions", ({ params }) => {
            perRowRequests.push(String(params.id));
            return HttpResponse.json(envelope([]));
          }),
        );
        await renderRows();
        for (const row of rows) {
          const verb = row.current_artifact_id
            ? "Action status unavailable"
            : "No result yet";
          expect(
            await within(
              screen.getByRole("button", {
                name: new RegExp(row.title),
                hidden: true,
              }),
            ).findByText(verb, { exact: true }),
          ).toBeTruthy();
        }
        expect(perRowRequests).toHaveLength(0);
      },
    );

    it("sends only ids and project_id with artifact ids in row order", async () => {
      let requestUrl: URL | undefined;
      server.use(
        http.get("*/v1/buddy/work", () =>
          HttpResponse.json(envelope(rows.map(record))),
        ),
        http.get("*/v1/buddy/work/actions", ({ request }) => {
          requestUrl = new URL(request.url);
          return HttpResponse.json(batchEnvelope(request));
        }),
      );
      await renderRows();
      await waitFor(() => expect(requestUrl).toBeDefined());
      if (!requestUrl) throw new Error("batch request was not captured");
      expect([...requestUrl.searchParams.keys()].sort()).toEqual([
        "ids",
        "project_id",
      ]);
      expect(requestUrl.searchParams.get("ids")).toBe(
        rows
          .filter((row) => row.current_artifact_id)
          .map((row) => row.id)
          .join(","),
      );
    });

    it("requests each appended page once without re-requesting prior ids", async () => {
      const firstPage = Array.from({ length: 50 }, (_, index) => ({
        ...item,
        id: `page-one-${index}`,
        title: `Page one report ${index}`,
        current_artifact_id: `page-one-artifact-${index}`,
      }));
      const secondPage = Array.from({ length: 3 }, (_, index) => ({
        ...item,
        id: `page-two-${index}`,
        title: `Page two report ${index}`,
        current_artifact_id: `page-two-artifact-${index}`,
      }));
      const batchIds: string[][] = [];
      server.use(
        http.get("*/v1/buddy/work", ({ request }) => {
          const cursor = new URL(request.url).searchParams.get("cursor");
          return HttpResponse.json(
            cursor
              ? envelope(secondPage.map(record))
              : envelope(firstPage.map(record), 50),
          );
        }),
        http.get("*/v1/buddy/work/actions", ({ request }) => {
          batchIds.push(
            new URL(request.url).searchParams.get("ids")?.split(",") ?? [],
          );
          return HttpResponse.json(batchEnvelope(request));
        }),
      );
      render(<BuddyDesk />, { preloadedState: config });
      await screen.findByRole("button", {
        name: /Page one report 0/,
        hidden: true,
      });
      await waitFor(() => expect(batchIds).toHaveLength(1));
      fireEvent.click(
        screen.getByRole("button", { name: "Load more reports" }),
      );
      await screen.findByRole("button", {
        name: /Page two report 0/,
        hidden: true,
      });
      await waitFor(() => expect(batchIds).toHaveLength(2));
      expect(batchIds).toEqual([
        firstPage.map((row) => row.id),
        secondPage.map((row) => row.id),
      ]);
    });

    it("marks a missing requested entry unavailable without affecting its siblings", async () => {
      const twoRows = [
        { ...item, title: "Present entry report" },
        { ...item, id: "missing-entry", title: "Missing entry report" },
      ];
      server.use(
        http.get("*/v1/buddy/work", () =>
          HttpResponse.json(envelope(twoRows.map(record))),
        ),
        http.get("*/v1/buddy/work/actions", () =>
          HttpResponse.json(
            envelope([{ work_id: item.id, actions: [action], error: null }]),
          ),
        ),
      );
      render(<BuddyDesk />, { preloadedState: config });
      const present = await screen.findByRole("button", {
        name: /Present entry report/,
        hidden: true,
      });
      expect(
        await within(present).findByText(action.label, { exact: true }),
      ).toBeTruthy();
      const missing = await screen.findByRole("button", {
        name: /Missing entry report/,
        hidden: true,
      });
      expect(
        await within(missing).findByText("Action status unavailable", {
          exact: true,
        }),
      ).toBeTruthy();
    });

    it("refetches the row batch after a successful action execution", async () => {
      const batchRequests: string[][] = [];
      server.use(
        http.get("*/v1/buddy/work", () =>
          HttpResponse.json(envelope([record(item)])),
        ),
        http.get("*/v1/buddy/work/:id/actions", () =>
          HttpResponse.json(envelope([action])),
        ),
        http.get("*/v1/buddy/work/actions", ({ request }) => {
          batchRequests.push(
            new URL(request.url).searchParams.get("ids")?.split(",") ?? [],
          );
          return HttpResponse.json(batchEnvelope(request, () => [action]));
        }),
        http.get("*/v1/buddy/artifacts/:id", () =>
          HttpResponse.json(envelope(record(artifact))),
        ),
        http.post(
          "*/v1/buddy/work/:id/actions/:action/execute",
          async ({ request }) => {
            const body = (await request.json()) as {
              idempotency_key: string;
            };
            return HttpResponse.json(
              envelope(record(receipt(body.idempotency_key))),
            );
          },
        ),
      );
      const { store } = render(<BuddyDesk />, { preloadedState: config });
      await waitFor(() => expect(batchRequests).toHaveLength(1));
      await openReport();
      await store.dispatch(
        buddyOperatorApi.endpoints.executeBuddyDeskAction.initiate({
          id: item.id,
          action_id: action.spec.action_id,
          project_id: project.project_id,
          canonical_root: project.canonical_root,
          work_revision: action.work_revision,
          artifact_id: action.artifact_id,
          artifact_revision: action.artifact_revision,
          spec_hash: action.spec_hash,
          idempotency_key: "batch-invalidation",
        }),
      );
      await waitFor(() => expect(batchRequests).toHaveLength(2));
    });
  });

  it("hides an action verb that would be repeated on every row", async () => {
    const second: WorkItem = {
      ...item,
      id: "work-2",
      title: "Second diagnosis",
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item), record(second)])),
      ),
      http.get("*/v1/buddy/work/actions", ({ request }) =>
        HttpResponse.json(
          batchEnvelope(request, (id) => [
            {
              ...action,
              spec: {
                ...action.spec,
                resource: {
                  ...action.spec.resource,
                  resource_id: id,
                },
              },
            },
          ]),
        ),
      ),
      http.get("*/v1/buddy/work/:id/actions", ({ params }) =>
        HttpResponse.json(
          envelope([
            {
              ...action,
              spec: {
                ...action.spec,
                resource: {
                  ...action.spec.resource,
                  resource_id: String(params.id),
                },
              },
            },
          ]),
        ),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const first = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    await screen.findByRole("button", {
      name: /Second diagnosis/,
      hidden: true,
    });
    await waitFor(() => expect(first).not.toHaveTextContent(action.label));
    expect(screen.queryByText(action.label, { exact: true })).toBeNull();
  });

  it("drops the state badge when the whole view shares one state and keeps it when it differentiates", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope([
            record(item),
            record({ ...item, id: "work-2", title: "Second diagnosis" }),
          ]),
        ),
      ),
    );
    const { unmount } = render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    expect(within(row).queryByText("Needs you", { exact: true })).toBeNull();
    unmount();
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope([
            record(item),
            record({
              ...item,
              id: "work-2",
              title: "Second diagnosis",
              state: "blocked",
            }),
          ]),
        ),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const mixed = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    expect(within(mixed).queryByText("Needs you", { exact: true })).toBeNull();
    const blocked = screen.getByRole("button", {
      name: /Second diagnosis/,
      hidden: true,
    });
    expect(within(blocked).getByText("Blocked", { exact: true })).toBeTruthy();
  });

  it("keeps the split rail stable and opens report detail only after selection", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    const css = await readGuiSource("features/Buddy/BuddyDesk.module.css");
    expect(readCssBlock(css, ".layout")).toContain(
      "grid-template-columns: minmax(0, 1fr);",
    );
    expect(css).toMatch(/@container \(min-width: \d+px\)/);
    expect(
      readCssBlock(
        readCssBlock(css, "@container (min-width: 880px)"),
        ".layout",
      ),
    ).toContain("grid-template-columns: minmax(300px, 23rem) minmax(0, 1fr);");
    render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    const desk = screen.getByTestId("buddy-desk-layout");
    expect(desk).toHaveAttribute("data-detail-open", "false");
    expect(screen.queryByRole("article")).toBeNull();
    fireEvent.click(row);
    expect(await screen.findByRole("article")).toBeTruthy();
    expect(desk).toHaveAttribute("data-detail-open", "true");
  });

  it("carries a machine-readable timestamp on the row without a hidden description", async () => {
    const timestampItem = { ...item, severity: "high", scope: "src/config" };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(timestampItem)])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const row = await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    const time = row.querySelector("time");
    expect(time).toHaveAttribute(
      "dateTime",
      new Date(timestampItem.last_material_change_at).toISOString(),
    );
    expect(row).not.toHaveAttribute("aria-describedby");
    expect(row).toHaveAccessibleName(/Provider setup diagnosis/);
    expect(row).toHaveAccessibleName(/High/);
    expect(row).toHaveAccessibleName(/src\/config/);
    expect(row).toHaveAccessibleName(/Changed /);
  });

  it("hides an opaque resource effect behind its resource kind", async () => {
    const run: RunOutcome = {
      schema_version: 1,
      run_id: "buddy-check-source-550e8400-e29b-41d4-a716-446655440000",
      work_id: item.id,
      parent_run_id: null,
      project,
      worker: { worker_id: "worker", engine_instance_id: "engine", epoch: 1 },
      origin: "autonomous",
      workflow: null,
      input_fingerprint: "saved-input",
      coverage: "The report was inspected.",
      started_at: "2026-09-05T00:00:00Z",
      ended_at: "2026-09-05T00:00:01Z",
      status: "completed",
      evidence: [],
      effects: [
        {
          resource: {
            resource_kind: "settings",
            resource_id: "producer:0123456789abcdef0123456789abcdef",
            version: "1",
          },
          state: "applied",
          evidence_refs: [],
        },
      ],
      failure_class: null,
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record({ ...item, state: "delivered" })])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.get("*/v1/buddy/work/:id/outcomes", () =>
        HttpResponse.json(envelope([record(run)])),
      ),
      http.get("*/v1/buddy/outcomes/:id", () =>
        HttpResponse.json(envelope(record(run))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(screen.getByRole("radio", { name: "Delivered" }));
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByText("The token has expired.");
    fireEvent.click(screen.getByRole("button", { name: /^Run history/ }));
    const details = screen.getByRole("region", { name: /^Run history/ });
    fireEvent.click(
      await within(details).findByRole("button", { name: /^Completed/ }),
    );
    const evidence = within(
      screen.getByRole("region", { name: "Recorded run" }),
    );
    expect(await evidence.findByText("Settings: Applied")).toBeTruthy();
    expect(evidence.queryByText(/producer:0123456789abcdef/)).toBeNull();
  });

  it("separates an archive that was never populated from an empty filter result", async () => {
    server.use(
      http.get("*/v1/buddy/work", ({ request }) =>
        HttpResponse.json(
          new URL(request.url).searchParams.get("search")
            ? envelope([])
            : envelope([record(item)]),
        ),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    await screen.findByRole("button", {
      name: /Provider setup diagnosis/,
      hidden: true,
    });
    fireEvent.click(screen.getByRole("radio", { name: "Delivered" }));
    expect(
      await screen.findByText("No delivered reports in this project yet."),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("radio", { name: "Needs you" }));
    fireEvent.change(await screen.findByRole("searchbox"), {
      target: { value: "nothing" },
    });
    expect(
      await screen.findByText("No reports match these filters."),
    ).toBeTruthy();
  });

  it("shows a one-line finding once and keeps its limitations", async () => {
    const oneLineArtifact = {
      ...artifact,
      title: "Default model not configured",
      conclusion: "Default model not configured",
      body: "Default model not configured",
      uncertainty:
        "The configured model list was not checked against the server.",
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(oneLineArtifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByRole("heading", {
      level: 3,
      name: "Default model not configured",
    });
    expect(screen.getAllByText("Default model not configured")).toHaveLength(1);
    expect(
      screen.getByText(
        "The configured model list was not checked against the server.",
      ),
    ).toBeTruthy();
    expect(screen.getByText("What this didn't cover")).toBeTruthy();
  });

  it("removes a repeated headline paragraph and keeps the rest of the write-up", async () => {
    const multilineArtifact = {
      ...artifact,
      title: "Default model not configured",
      conclusion: "Default model not configured",
      body: "Default model not configured\n\nMore context about the missing model.\n\nSuggested next steps:\n- Draft a skill: Safer shell",
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(multilineArtifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    await screen.findByRole("heading", {
      level: 3,
      name: "Default model not configured",
    });
    expect(screen.getAllByText("Default model not configured")).toHaveLength(1);
    expect(
      screen.getByText("More context about the missing model."),
    ).toBeTruthy();
    expect(screen.getByText("Draft a skill: Safer shell")).toBeTruthy();
  });

  it("shows a workflow conclusion when it adds information beyond the title", async () => {
    const workflowArtifact = {
      ...artifact,
      title: "Dependency review",
      conclusion: "Three dependencies have known vulnerabilities.",
      body: "The report covers the current lockfile.",
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(workflowArtifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    expect(
      await screen.findByRole("heading", {
        level: 3,
        name: "Dependency review",
      }),
    ).toBeTruthy();
    expect(
      screen.getByText("Three dependencies have known vulnerabilities."),
    ).toBeTruthy();
  });

  it("groups loaded records by typed subject without title-only merging and keeps every record reachable", async () => {
    const sameSubject = (id: string, title: string) => ({
      ...item,
      id,
      title,
      state: "delivered" as const,
      current_artifact_id: null,
      current_artifact_revision: null,
    });
    const rows = [
      sameSubject("work-latest", "Provider health latest"),
      sameSubject("work-older", "Provider health older"),
      {
        ...sameSubject("work-same-title", "Provider health latest"),
        subject: { ...item.subject, resource_id: "another subject" },
      },
      {
        ...sameSubject("work-other-kind", "Provider health other kind"),
        kind: "review",
      },
    ];
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope(rows.map(record), 7)),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(await screen.findByRole("radio", { name: "Delivered" }));
    await screen.findByRole("button", {
      name: /Provider health other kind/,
      hidden: true,
    });
    expect(screen.getByTestId("buddy-desk-count")).toHaveTextContent(
      "4 reports · 3 subjects · more available",
    );
    expect(
      screen.getAllByRole("button", {
        name: /Provider health latest/,
        hidden: true,
      }),
    ).toHaveLength(2);
    const trigger = screen.getByRole("button", {
      name: "1 earlier report for this subject",
    });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    const history = document.getElementById(
      trigger.getAttribute("aria-controls") ?? "",
    );
    if (!history) throw new Error("history disclosure missing");
    expect(history).not.toBeVisible();
    expect(
      within(history).getByRole("button", {
        name: /Provider health older/,
        hidden: true,
      }),
    ).toBeTruthy();
    expect(screen.queryByText(/superseded|replaced/i)).toBeNull();
    fireEvent.click(screen.getByRole("radio", { name: "Run history" }));
    expect(screen.getByRole("radio", { name: "Run history" })).toBeChecked();
    expect(
      screen
        .getAllByTestId("buddy-desk-row-title")
        .map((title) => title.textContent),
    ).toEqual(rows.map((row) => row.title));
  });

  it("separates cancelled records from delivered results inside Delivered without dropping them", async () => {
    const rows = [
      {
        ...item,
        id: "work-delivered",
        title: "Delivered review",
        state: "delivered" as const,
      },
      {
        ...item,
        id: "work-cancel-1",
        title: "Cancelled check one",
        state: "cancelled" as const,
        subject: { ...item.subject, resource_id: "one" },
      },
      {
        ...item,
        id: "work-cancel-2",
        title: "Cancelled check two",
        state: "cancelled" as const,
        subject: { ...item.subject, resource_id: "two" },
      },
    ];
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope(rows.map(record))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(await screen.findByRole("radio", { name: "Delivered" }));
    await screen.findByRole("button", {
      name: /Delivered review/,
      hidden: true,
    });
    const label = "Cancelled · 2 reports kept for history";
    const cancelledTrigger = screen.getByRole("button", { name: label });
    expect(cancelledTrigger).toHaveAttribute("aria-expanded", "false");
    const cancelled = document.getElementById(
      cancelledTrigger.getAttribute("aria-controls") ?? "",
    );
    if (!cancelled) throw new Error("cancelled disclosure missing");
    expect(
      within(cancelled).getAllByRole("button", {
        name: /Cancelled check/,
        hidden: true,
      }),
    ).toHaveLength(2);
    expect(
      within(cancelled).queryByRole("button", {
        name: /Delivered review/,
        hidden: true,
      }),
    ).toBeNull();
    expect(screen.getByTestId("buddy-desk-count")).toHaveTextContent(
      "3 reports · 1 subject · 2 cancelled",
    );
  });

  it("keeps the work list in a bounded scroll region instead of a tall fixed desk", async () => {
    const css = await readGuiSource("features/Buddy/BuddyDesk.module.css");
    expect(readCssBlock(css, ".desk")).not.toContain("min-height");
    expect(readCssBlock(css, ".scroller")).toContain("max-height: 70vh;");
    expect(readCssBlock(css, ".scroller")).toContain("overflow-y: auto;");
    expect(readCssBlock(css, ".scroller")).toContain("mask-image");
    expect(readCssBlock(css, ".items")).not.toContain("max-width");
    expect(css).toContain("@container (min-width: 880px)");
    expect(css).not.toMatch(/@media/);
  });

  it("shows the finding and write-up before report management controls", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([action])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    const finding = await screen.findByRole("region", { name: "Finding" });
    const manage = screen.getByRole("region", { name: "Manage this report" });
    await within(manage).findByRole("region", { name: "Report actions" });
    const writeUp = screen.getByRole("button", { name: "Full write-up" });
    const evidence = screen.getByRole("button", { name: /^Evidence/ });
    const follows = (a: Node, b: Node) =>
      Boolean(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);
    expect(follows(finding, writeUp)).toBe(true);
    expect(follows(writeUp, evidence)).toBe(true);
    expect(follows(evidence, manage)).toBe(true);
    expect(
      within(manage).getByRole("region", { name: "Report reading status" }),
    ).toBeTruthy();
    expect(
      within(manage).getByRole("region", { name: "Work snooze" }),
    ).toBeTruthy();
    fireEvent.click(evidence);
    expect(
      await screen.findByText(/it doesn't prove the finding is correct/),
    ).toBeTruthy();
  });

  it("refuses an artifact whose identity does not match the selected report revision", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope([record({ ...item, current_artifact_revision: null })]),
        ),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(
          envelope(record({ ...artifact, artifact_id: "artifact-other" })),
        ),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Provider setup diagnosis/,
        hidden: true,
      }),
    );
    const detail = await screen.findByRole("article");
    await waitFor(() => expect(within(detail).getByRole("alert")).toBeTruthy());
    expect(screen.queryByText("The token has expired.")).toBeNull();
    expect(
      screen.queryByRole("region", { name: "Manage this report" }),
    ).toBeNull();
  });

  it("explains a deep link without a project instead of opening anything", async () => {
    const linkRequests: string[] = [];
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/work/work-1", ({ request }) => {
        linkRequests.push(request.url);
        return HttpResponse.json(envelope(record(item)));
      }),
    );
    render(<BuddyDesk workId={item.id} />, { preloadedState: config });
    expect(
      await screen.findByText(/This link doesn't name a project/),
    ).toBeTruthy();
    expect(screen.queryByRole("article")).toBeNull();
    expect(linkRequests).toHaveLength(0);
  });

  it("refuses a deep-linked work item whose record belongs to another project root", async () => {
    server.use(
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
      http.get("*/v1/buddy/work/work-1", () =>
        HttpResponse.json(
          envelope(
            record({
              ...item,
              project: { ...project, canonical_root: "/scratch/other" },
            }),
          ),
        ),
      ),
    );
    render(<BuddyDesk workId={item.id} projectId={project.project_id} />, {
      preloadedState: config,
    });
    expect(await screen.findByText(/No other report was opened/)).toBeTruthy();
    expect(screen.queryByRole("article")).toBeNull();
  });

  it("opens the collapsed history group when a deep link selects an earlier loaded record", async () => {
    const older = {
      ...item,
      id: "work-older",
      title: "Older provider record",
    };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(
          envelope([
            record({
              ...item,
              id: "work-latest",
              title: "Latest provider record",
            }),
            record(older),
          ]),
        ),
      ),
      http.get("*/v1/buddy/work/work-older", () =>
        HttpResponse.json(envelope(record(older))),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    render(<BuddyDesk workId={older.id} projectId={project.project_id} />, {
      preloadedState: config,
    });
    expect(await screen.findByText("The token has expired.")).toBeTruthy();
    const row = await screen.findByRole("button", {
      name: /Older provider record/,
      hidden: true,
    });
    await waitFor(() =>
      expect(row.closest('[role="region"]')).not.toHaveAttribute("hidden"),
    );
    expect(row).toHaveAttribute("aria-current", "true");
  });

  it("keeps the same scroller, row nodes and batch subscriptions when more than one action batch loads", async () => {
    const rowsFor = (from: number, count: number) =>
      Array.from({ length: count }, (_, index) => ({
        ...item,
        id: `paged-work-${from + index}`,
        title: `Paged report ${from + index}`,
        subject: { ...item.subject, resource_id: `subject ${from + index}` },
        current_artifact_id: `paged-artifact-${from + index}`,
      }));
    const batches: string[][] = [];
    server.use(
      http.get("*/v1/buddy/work", ({ request }) => {
        const cursor = new URL(request.url).searchParams.get("cursor");
        return HttpResponse.json(
          cursor
            ? envelope(rowsFor(50, 50).map(record))
            : envelope(rowsFor(0, 50).map(record), 50),
        );
      }),
      http.get("*/v1/buddy/work/actions", ({ request }) => {
        batches.push(
          new URL(request.url).searchParams.get("ids")?.split(",") ?? [],
        );
        return HttpResponse.json(batchEnvelope(request));
      }),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const first = await screen.findByRole("button", {
      name: /Paged report 0\b/,
      hidden: true,
    });
    const scroller = screen.getByTestId("buddy-desk-scroller");
    await waitFor(() => expect(batches).toHaveLength(1));
    fireEvent.click(screen.getByRole("button", { name: "Load more reports" }));
    await screen.findByRole("button", {
      name: /Paged report 99\b/,
      hidden: true,
    });
    await waitFor(() => expect(batches).toHaveLength(2));
    expect(screen.getByTestId("buddy-desk-scroller")).toBe(scroller);
    expect(
      screen.getByRole("button", { name: /Paged report 0\b/, hidden: true }),
    ).toBe(first);
    expect(batches[1]).toEqual(rowsFor(50, 50).map((row) => row.id));
    expect(
      within(scroller).getAllByTestId("buddy-desk-row-title"),
    ).toHaveLength(100);
    fireEvent.click(first);
    fireEvent.click(
      await screen.findByRole("button", { name: "Back to reports" }),
    );
    await waitFor(() => expect(document.activeElement).toBe(first));
    expect(batches).toHaveLength(2);
  });
});
