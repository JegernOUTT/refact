import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it } from "vitest";
import { setupListeners } from "@reduxjs/toolkit/query";
import { fireEvent, render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { setUpStore } from "../../app/store";
import { BuddyDeskActions } from "./BuddyDeskActions";
import {
  buddyOperatorIdentity,
  type DeskAction,
} from "../../services/refact/buddyOperator";
import type {
  ActionExecution,
  ArtifactRevision,
  ProjectIdentity,
  WorkItem,
} from "./operatorTypes";

const project: ProjectIdentity = {
  project_id: "project",
  canonical_root: "/scratch/project",
};
const foreignRoot: ProjectIdentity = {
  project_id: "project",
  canonical_root: "/scratch/other",
};
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
const dismissal = (key: string): ActionExecution => ({
  schema_version: 1,
  execution_id: "dismiss-execution",
  project,
  worker: { worker_id: "worker", engine_instance_id: "engine", epoch: 1 },
  idempotency_key: key,
  action_id: dismissAction.spec.action_id,
  spec_hash: dismissAction.spec_hash,
  requester: {
    source_kind: "external",
    source_id: "buddy-desk:dismiss",
    source_version: "1",
  },
  decision: null,
  status: "completed",
  started_at: "2026-09-05T00:00:00Z",
  ended_at: "2026-09-05T00:00:01Z",
  effects: [],
  verification_evidence: ["Work item dismissed by user request."],
  recovery: null,
});
const envelope = <T,>(data: T, envelopeProject = project) => ({
  schema_version: 1,
  project: envelopeProject,
  data,
  next_cursor: null,
});
const actionButton = { name: action.label };
const emptyState = /No actions are available for this report revision/;

function renderActions() {
  const store = setUpStore(config);
  const identity = buddyOperatorIdentity(store.getState());
  return render(
    <BuddyDeskActions
      item={item}
      artifact={artifact}
      endpointIdentity={identity}
    />,
    { store },
  );
}

describe("BuddyDeskActions", () => {
  let stopListeners: (() => void) | undefined;
  afterEach(() => {
    stopListeners?.();
    stopListeners = undefined;
  });

  it("renders a registered action bound to this project root, artifact and revision", async () => {
    server.use(
      http.get("*/v1/buddy/work/:id/actions", ({ request }) => {
        expect(new URL(request.url).searchParams.get("project_id")).toBe(
          project.project_id,
        );
        return HttpResponse.json(envelope([action]));
      }),
    );
    renderActions();
    expect(await screen.findByRole("button", actionButton)).toBeTruthy();
    expect(screen.getByText(action.description)).toBeTruthy();
    expect(screen.queryByText(emptyState)).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps the primary action stacked and lays lifecycle and secondary controls out in one row", async () => {
    server.use(
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([action, dismissAction])),
      ),
    );
    const store = setUpStore(config);
    render(
      <BuddyDeskActions
        item={item}
        artifact={artifact}
        endpointIdentity={buddyOperatorIdentity(store.getState())}
        secondary={<button type="button">Mark as read</button>}
      />,
      { store },
    );
    const primary = await screen.findByRole("button", actionButton);
    expect(screen.getByText(action.description).tagName).toBe("P");
    const dismiss = screen.getByRole("button", { name: dismissAction.label });
    expect(dismiss).toHaveAccessibleDescription(dismissAction.description);
    const row = screen.getByRole("button", {
      name: "Mark as read",
    }).parentElement;
    if (!row) throw new Error("Expected the secondary actions row");
    expect(row).toContainElement(dismiss);
    expect(row).toContainElement(
      screen.getByRole("button", { name: "Refresh actions" }),
    );
    expect(row).not.toContainElement(primary);
  });

  it("does not render an action registered against another project root", async () => {
    server.use(
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(
          envelope(
            [{ ...action, spec: { ...action.spec, project: foreignRoot } }],
            foreignRoot,
          ),
        ),
      ),
    );
    renderActions();
    expect(await screen.findByText(emptyState)).toBeTruthy();
    expect(screen.queryByRole("button", actionButton)).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("does not render an action bound to a superseded report revision", async () => {
    server.use(
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([{ ...action, artifact_revision: 2 }])),
      ),
    );
    renderActions();
    expect(await screen.findByText(emptyState)).toBeTruthy();
    expect(screen.queryByRole("button", actionButton)).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("reports a failed actions request instead of an empty action list", async () => {
    server.use(
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json({ detail: "unavailable" }, { status: 503 }),
      ),
    );
    renderActions();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't load actions for this report.",
    );
    expect(screen.queryByText(emptyState)).toBeNull();
    expect(screen.queryByRole("button", actionButton)).toBeNull();
  });

  it("replaces rather than duplicates rendered actions when a reconnect refetch resolves", async () => {
    let requests = 0;
    const refetched = {
      ...action,
      description: "Refetched saved evidence description.",
    };
    server.use(
      http.get("*/v1/buddy/work/:id/actions", () => {
        requests += 1;
        return HttpResponse.json(
          envelope([requests === 1 ? action : refetched]),
        );
      }),
    );
    const { store } = renderActions();
    stopListeners = setupListeners(store.dispatch);
    expect(await screen.findByRole("button", actionButton)).toBeTruthy();
    await waitFor(() => expect(requests).toBe(1));
    fireEvent(window, new Event("online"));
    await waitFor(() => expect(requests).toBe(2));
    expect(await screen.findByText(refetched.description)).toBeTruthy();
    expect(screen.queryByText(action.description)).toBeNull();
    expect(screen.getAllByRole("button", actionButton)).toHaveLength(1);
  });

  it("renders dismissal as supported and names dismissal in pending and receipt copy", async () => {
    let release: (() => void) | undefined;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    server.use(
      http.get("*/v1/buddy/work/:id/actions", () =>
        HttpResponse.json(envelope([dismissAction])),
      ),
      http.post(
        "*/v1/buddy/work/:id/actions/:action/execute",
        async ({ params, request }) => {
          expect(params.action).toBe("dismiss_not_a_problem");
          const body = (await request.json()) as { idempotency_key: string };
          await pending;
          return HttpResponse.json(
            envelope({
              value: dismissal(body.idempotency_key),
              revision: 1,
              change_seq: 2,
            }),
          );
        },
      ),
    );
    renderActions();
    const button = await screen.findByRole("button", {
      name: dismissAction.label,
    });
    expect(button).not.toBeDisabled();
    expect(screen.queryByText(/not supported by this client/)).toBeNull();
    fireEvent.click(button);
    expect(
      await screen.findByRole("button", { name: "Dismissing item…" }),
    ).toBeTruthy();
    expect(screen.queryByText(/saved evidence/i)).toBeNull();
    release?.();
    expect(
      await screen.findByText(/Dismissed from Needs you: Completed/i),
    ).toBeTruthy();
    expect(screen.queryByText(/saved evidence/i)).toBeNull();
  });
});
