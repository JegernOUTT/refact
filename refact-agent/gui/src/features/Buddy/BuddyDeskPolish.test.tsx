import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  within,
} from "../../utils/test-utils";
import { act } from "react-dom/test-utils";
import { server } from "../../utils/mockServer";
import { setUpStore } from "../../app/store";
import { buddyOperatorIdentity } from "../../services/refact/buddyOperator";
import { BuddyDesk } from "./BuddyDesk";
import type { ArtifactRevision, WorkItem } from "./operatorTypes";
import { resetBuddyPersistentState, writeBuddyPersistentState } from "./ui";
import { BUDDY_REPORTS_SELECTION_KEY } from "./sections/buddyReportsLink";
import { DESK_SEARCH_DEBOUNCE_MS } from "./hooks/useDeskWorkList";

const project = { project_id: "project-1", canonical_root: "/repo" };
const config = {
  config: { lspPort: 8001, host: "vscode" as const, themeProps: {} },
};
const item: WorkItem = {
  schema_version: 1,
  id: "work-1",
  project,
  subject: { resource_kind: "check", resource_id: "providers", version: "1" },
  kind: "diagnosis",
  title: "Provider setup diagnosis",
  state: "needs_attention",
  native_sources: [],
  current_artifact_id: "artifact-1",
  current_artifact_revision: 1,
  last_material_change_at: "2026-09-05T10:00:00Z",
};
const offPage: WorkItem = {
  ...item,
  id: "work-9",
  title: "Report beyond the first page",
  subject: { ...item.subject, resource_id: "models" },
};
const artifact: ArtifactRevision = {
  schema_version: 1,
  artifact_id: "artifact-1",
  revision: 1,
  project,
  source_run_ids: ["run-1"],
  title: "Connection assessment",
  conclusion: "The token has expired.",
  delta: "",
  body: "Reconnect the provider.",
  uncertainty: null,
  proposed_action_ids: [],
  evidence: [],
  provenance: {
    source_run_ids: ["run-1"],
    evidence_refs: [],
    contains_untrusted_source: false,
  },
};
const envelope = <T,>(data: T, next_cursor: number | null = null) => ({
  schema_version: 1,
  project,
  data,
  next_cursor,
});
const record = <T,>(value: T) => ({ value, revision: 1, change_seq: 1 });

function restoreSelection(workId: string) {
  const store = setUpStore(config);
  writeBuddyPersistentState(BUDDY_REPORTS_SELECTION_KEY, {
    workId,
    projectId: project.project_id,
    endpointIdentity: buddyOperatorIdentity(store.getState()),
    view: "needs_you",
  });
  return store;
}

describe("BuddyDesk polish", () => {
  beforeEach(() => resetBuddyPersistentState());

  it("shows the detail loading state while a restored report outside the page loads", async () => {
    let release: () => void = () => undefined;
    const held = new Promise<void>((resolve) => (release = resolve));
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/work/actions", () =>
        HttpResponse.json(envelope([])),
      ),
      http.get("*/v1/buddy/work/:id", async () => {
        await held;
        return HttpResponse.json(envelope(record(offPage)));
      }),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    const store = restoreSelection(offPage.id);
    render(<BuddyDesk />, { store });
    expect(
      await screen.findByRole("status", { name: "Opening the report" }),
    ).toBeTruthy();
    expect(screen.getByTestId("buddy-desk-layout")).toHaveAttribute(
      "data-detail-visible",
      "true",
    );
    release();
    expect(await screen.findByText("The token has expired.")).toBeTruthy();
    expect(screen.getByRole("article")).toBeTruthy();
    expect(
      screen.queryByRole("status", { name: "Opening the report" }),
    ).toBeNull();
  });

  it("clears a restored selection whose report is gone and falls back to the list", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/work/actions", () =>
        HttpResponse.json(envelope([])),
      ),
      http.get("*/v1/buddy/work/:id", () =>
        HttpResponse.json({ detail: "missing" }, { status: 404 }),
      ),
    );
    const store = restoreSelection("gone-work");
    render(<BuddyDesk />, { store });
    expect(
      await screen.findByText(
        "Couldn't reopen the last report you were reading.",
      ),
    ).toBeTruthy();
    expect(
      screen.queryByRole("status", { name: "Opening the report" }),
    ).toBeNull();
    expect(screen.getByTestId("buddy-desk-detail-hint")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /Provider setup diagnosis/ }),
    ).toBeTruthy();
  });

  it("reports an unread-marker failure without breaking the list", async () => {
    server.use(
      http.get("*/v1/buddy/work", ({ request }) =>
        new URL(request.url).searchParams.get("attention") === "unread"
          ? HttpResponse.json({ detail: "down" }, { status: 503 })
          : HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/work/actions", () =>
        HttpResponse.json(envelope([])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    expect(
      await screen.findByRole("button", { name: "Retry unread markers" }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /Provider setup diagnosis/ }),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Retry reports" })).toBeNull();
  });

  it("hides the detail hint next to an empty or failed list", async () => {
    server.use(
      http.get("*/v1/buddy/work", ({ request }) =>
        new URL(request.url).searchParams.get("view") === "delivered"
          ? HttpResponse.json({ detail: "down" }, { status: 503 })
          : HttpResponse.json(envelope([])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: "Browse reports" }),
    );
    fireEvent.click(await screen.findByRole("radio", { name: "Working on" }));
    await screen.findByText("Nothing is in progress right now.");
    expect(screen.queryByTestId("buddy-desk-detail-hint")).toBeNull();
    fireEvent.click(screen.getByRole("radio", { name: "Delivered" }));
    await screen.findByRole("button", { name: "Retry reports" });
    expect(screen.queryByTestId("buddy-desk-detail-hint")).toBeNull();
  });

  it("debounces search: no request before 250ms, exactly one after", async () => {
    const searches: (string | null)[] = [];
    server.use(
      http.get("*/v1/buddy/work", ({ request }) => {
        const params = new URL(request.url).searchParams;
        if (params.get("attention") !== "unread")
          searches.push(params.get("search"));
        return HttpResponse.json(envelope([record(item)]));
      }),
      http.get("*/v1/buddy/work/actions", () =>
        HttpResponse.json(envelope([])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    await screen.findByRole("button", { name: /Provider setup diagnosis/ });
    const before = searches.length;
    const box = screen.getByRole("searchbox", { name: "Search reports" });
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const settle = async (ms: number) => {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(ms);
        for (let tick = 0; tick < 50; tick += 1) await Promise.resolve();
      });
    };
    try {
      for (const value of ["p", "pr", "pro", "prov"])
        fireEvent.change(box, { target: { value } });
      await settle(DESK_SEARCH_DEBOUNCE_MS - 1);
      expect(searches.length).toBe(before);
      await settle(1);
    } finally {
      vi.useRealTimers();
    }
    await waitFor(() => expect(searches.length).toBe(before + 1));
    expect(searches.at(-1)).toBe("prov");
    await new Promise((resolve) =>
      setTimeout(resolve, DESK_SEARCH_DEBOUNCE_MS * 2),
    );
    expect(searches.length).toBe(before + 1);
  });

  it("shows skeletons for an uncached key but keeps stale rows on a same-key refetch", async () => {
    let releaseDelivered: () => void = () => undefined;
    const delivered = new Promise<void>(
      (resolve) => (releaseDelivered = resolve),
    );
    let releaseRefresh: () => void = () => undefined;
    let holdRefresh: Promise<void> | null = null;
    server.use(
      http.get("*/v1/buddy/work", async ({ request }) => {
        const params = new URL(request.url).searchParams;
        if (params.get("view") === "delivered") {
          await delivered;
          return HttpResponse.json(
            envelope([record({ ...item, id: "work-3", state: "delivered" })]),
          );
        }
        if (holdRefresh && params.get("attention") !== "unread")
          await holdRefresh;
        return HttpResponse.json(envelope([record(item)]));
      }),
      http.get("*/v1/buddy/work/actions", () =>
        HttpResponse.json(envelope([])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    await screen.findByRole("button", { name: /Provider setup diagnosis/ });

    holdRefresh = new Promise<void>((resolve) => (releaseRefresh = resolve));
    const refresh = screen.getByRole("button", { name: "Refresh reports" });
    fireEvent.click(refresh);
    expect(await screen.findByRole("status", { name: "" })).toHaveTextContent(
      "Refreshing reports…",
    );
    expect(refresh).toHaveAttribute("aria-busy", "true");
    expect(
      screen.queryByRole("status", { name: "Loading reports" }),
    ).toBeNull();
    expect(
      screen.getByRole("button", { name: /Provider setup diagnosis/ }),
    ).toBeTruthy();
    releaseRefresh();
    holdRefresh = null;
    await waitFor(() => expect(refresh).not.toHaveAttribute("aria-busy"));
    expect(screen.queryByText("Refreshing reports…")).toBeNull();

    fireEvent.click(screen.getByRole("radio", { name: "Delivered" }));
    expect(
      await screen.findByRole("status", { name: "Loading reports" }),
    ).toBeTruthy();
    expect(
      screen.queryByText("No delivered reports in this project yet."),
    ).toBeNull();
    releaseDelivered();
    await waitFor(() =>
      expect(
        screen.queryByRole("status", { name: "Loading reports" }),
      ).toBeNull(),
    );
  });

  it("keeps the grouping explanation as a description, not the button name", async () => {
    const repeat = { ...item, id: "work-2", title: "Older provider check" };
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item), record(repeat)])),
      ),
      http.get("*/v1/buddy/work/actions", () =>
        HttpResponse.json(envelope([])),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    const info = await screen.findByRole("button", { name: "About grouping" });
    expect(info).toHaveAccessibleDescription(/grouped when they share/);
    const desk = screen.getByTestId("buddy-desk");
    const list = within(desk).getByRole("list", { name: "Reports" });
    const head = within(list).getByRole("button", {
      name: /Provider setup diagnosis/,
    });
    expect(head.closest("ul")).toBe(list);
    expect(head.closest("li")?.parentElement).toBe(list);
  });

  it("hides empty evidence and earlier-revision disclosures", async () => {
    server.use(
      http.get("*/v1/buddy/work", () =>
        HttpResponse.json(envelope([record(item)])),
      ),
      http.get("*/v1/buddy/work/actions", () =>
        HttpResponse.json(envelope([])),
      ),
      http.get("*/v1/buddy/artifacts/:id", () =>
        HttpResponse.json(envelope(record(artifact))),
      ),
    );
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: /Provider setup diagnosis/ }),
    );
    await screen.findByText("The token has expired.");
    expect(screen.queryByRole("button", { name: /^Evidence/ })).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Earlier revisions" }),
    ).toBeNull();
    expect(screen.getByRole("button", { name: /^Run history/ })).toBeTruthy();
  });
});
