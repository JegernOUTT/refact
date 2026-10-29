import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";
import { render, screen, fireEvent, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BuddyDesk } from "./BuddyDesk";
import type { WorkItem } from "./operatorTypes";
import {
  readBuddyPersistentState,
  resetBuddyPersistentState,
  writeBuddyPersistentState,
} from "./ui";
import {
  BUDDY_REPORTS_SELECTION_KEY,
  BUDDY_REPORTS_VIEW_KEY,
  requestBuddyReportsView,
} from "./sections/buddyReportsLink";

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
  current_artifact_id: null,
  current_artifact_revision: null,
  last_material_change_at: "2026-09-05T10:00:00Z",
};
const second: WorkItem = {
  ...item,
  id: "work-2",
  title: "Second diagnosis",
  subject: { ...item.subject, resource_id: "models" },
};
const delivered: WorkItem = {
  ...item,
  id: "work-3",
  title: "Delivered review",
  state: "delivered",
};
const envelope = <T,>(data: T) => ({
  schema_version: 1,
  project,
  data,
  next_cursor: null,
});
const record = <T,>(value: T) => ({ value, revision: 1, change_seq: 1 });

function useList(unreadIds: string[] = []) {
  const views: (string | null)[] = [];
  server.use(
    http.get("*/v1/buddy/work", ({ request }) => {
      const params = new URL(request.url).searchParams;
      const view = params.get("view");
      views.push(view);
      const rows = [item, second, delivered].filter((row) =>
        view === "delivered"
          ? row.state === "delivered"
          : row.state !== "delivered",
      );
      const visible =
        params.get("attention") === "unread"
          ? rows.filter((row) => unreadIds.includes(row.id))
          : rows;
      return HttpResponse.json(envelope(visible.map(record)));
    }),
    http.get("*/v1/buddy/work/actions", () => HttpResponse.json(envelope([]))),
    http.get("*/v1/buddy/work/:id", ({ params }) => {
      const found = [item, second, delivered].find(
        (row) => row.id === params.id,
      );
      return found
        ? HttpResponse.json(envelope(record(found)))
        : HttpResponse.json({ detail: "missing" }, { status: 404 });
    }),
  );
  return views;
}

describe("BuddyDesk persistent state", () => {
  beforeEach(() => resetBuddyPersistentState());

  it("shows a plain detail hint without auto-selecting or fetching report details", async () => {
    useList();
    const detailRequests: string[] = [];
    server.use(
      http.get("*/v1/buddy/work/:id", ({ request }) => {
        detailRequests.push(request.url);
        return HttpResponse.json(envelope(record(item)));
      }),
      http.get("*/v1/buddy/work/:id/read", ({ request }) => {
        detailRequests.push(request.url);
        return HttpResponse.json(envelope(null));
      }),
      http.post("*/v1/buddy/work/:id/read", ({ request }) => {
        detailRequests.push(request.url);
        return HttpResponse.json(envelope(null));
      }),
    );
    render(<BuddyDesk />, { preloadedState: config });
    await screen.findByRole("button", { name: /Provider setup diagnosis/ });
    const hint = screen.getByTestId("buddy-desk-detail-hint");
    expect(hint).toHaveTextContent("Pick a report to read it here");
    expect(
      hint.querySelector("button, a, [tabindex], [aria-live], [role]"),
    ).toBeNull();
    expect(detailRequests).toEqual([]);
    expect(screen.queryByRole("article")).toBeNull();
    expect(screen.getByTestId("buddy-desk-layout")).toHaveAttribute(
      "data-detail-open",
      "false",
    );
    expect(readBuddyPersistentState(BUDDY_REPORTS_SELECTION_KEY)).toBe(
      undefined,
    );
  });

  it("opens the view requested from Home", async () => {
    const views = useList();
    requestBuddyReportsView("delivered");
    render(<BuddyDesk />, { preloadedState: config });
    expect(
      await screen.findByRole("radio", { name: "Delivered" }),
    ).toBeChecked();
    await screen.findByRole("button", { name: /Delivered review/ });
    expect(views[0]).toBe("delivered");
  });

  it("keeps the view and selected report across remounts", async () => {
    useList();
    const first = render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: /Second diagnosis/ }),
    );
    await screen.findByRole("article");
    expect(screen.queryByTestId("buddy-desk-detail-hint")).toBeNull();
    expect(readBuddyPersistentState(BUDDY_REPORTS_SELECTION_KEY)).toMatchObject(
      { workId: second.id, view: "needs_you" },
    );
    first.unmount();
    render(<BuddyDesk />, { preloadedState: config });
    expect(
      await screen.findByRole("heading", {
        level: 3,
        name: "Second diagnosis",
      }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /Second diagnosis/, hidden: true }),
    ).toHaveAttribute("aria-current", "true");
    fireEvent.click(screen.getByRole("button", { name: "Back to reports" }));
    expect(readBuddyPersistentState(BUDDY_REPORTS_SELECTION_KEY)).toBeNull();
  });

  it("lets a deep link win over a stale stored selection", async () => {
    useList();
    writeBuddyPersistentState(BUDDY_REPORTS_SELECTION_KEY, {
      workId: second.id,
      projectId: project.project_id,
      endpointIdentity: "stale",
      view: "needs_you",
    });
    render(<BuddyDesk workId={item.id} projectId={project.project_id} />, {
      preloadedState: config,
    });
    expect(
      await screen.findByRole("heading", {
        level: 3,
        name: "Provider setup diagnosis",
      }),
    ).toBeTruthy();
    expect(
      screen.queryByRole("heading", { level: 3, name: "Second diagnosis" }),
    ).toBeNull();
  });

  it("marks only unread reports with the unread indicator", async () => {
    useList([second.id]);
    render(<BuddyDesk />, { preloadedState: config });
    const unread = await screen.findByRole("button", {
      name: /Second diagnosis/,
    });
    await waitFor(() => expect(unread.closest("[data-unread]")).not.toBeNull());
    expect(
      screen
        .getByRole("button", { name: /Provider setup diagnosis/ })
        .closest("[data-unread]"),
    ).toBeNull();
  });

  it("switches views with radio semantics and drops the stale selection", async () => {
    useList();
    render(<BuddyDesk />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: /Provider setup diagnosis/ }),
    );
    await screen.findByRole("article");
    fireEvent.click(screen.getByRole("radio", { name: "Delivered" }));
    await screen.findByRole("button", { name: /Delivered review/ });
    expect(screen.queryByRole("article")).toBeNull();
    expect(readBuddyPersistentState(BUDDY_REPORTS_VIEW_KEY)).toBe("delivered");
    expect(readBuddyPersistentState(BUDDY_REPORTS_SELECTION_KEY)).toBeNull();
  });
});
