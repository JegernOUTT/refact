import { render, screen, waitFor } from "../../utils/test-utils";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { server } from "../../utils/mockServer";
import { setUpStore } from "../../app/store";
import { BuddyPublicationRecovery } from "./BuddyPublicationRecovery";
import {
  buddyOperatorIdentity,
  parsePendingPublications,
  type OperatorStatus,
} from "../../services/refact/buddyOperator";
const status: OperatorStatus = {
  schema_version: 1,
  mode: "enabled",
  project: { project_id: "project", canonical_root: "/project" },
  worker: { worker_id: "worker", engine_instance_id: "engine", epoch: 1 },
  features: {
    storage: true,
    actions: true,
    supervision: true,
    improvement: true,
    editorial: true,
  },
};

describe("publication-only recovery", () => {
  it("keeps recovery failure and no-rerun guidance in an accessible status", async () => {
    const store = setUpStore();
    server.use(
      http.post("*/v1/buddy/operator/pending-publications", () =>
        HttpResponse.json({ error: "Storage unavailable" }, { status: 503 }),
      ),
    );
    render(
      <BuddyPublicationRecovery
        status={status}
        identity={buddyOperatorIdentity(store.getState())}
      />,
      { store },
    );

    const warning = await screen.findByRole("status");
    expect(warning.textContent).toContain(
      "Publication recovery is unavailable",
    );
    expect(warning.textContent).toContain("do not rerun side effects");
    expect(warning.textContent).toContain("memory-only");
    expect(warning.textContent).toContain("lost on engine restart");
    expect(screen.queryByRole("button", { name: /Retry saving/ })).toBeNull();
  });

  it("discovers metadata, warns about restart, and clears successful publication retry", async () => {
    const store = setUpStore();
    let requestBody: unknown;
    const envelope = (data: unknown) =>
      HttpResponse.json({ schema_version: 1, project: status.project, data });
    server.use(
      http.post("*/v1/buddy/operator/pending-publications", () =>
        envelope([
          {
            run_id: "run",
            workflow_id: "commit_message",
            started_at: "2026-09-05T00:00:00Z",
          },
        ]),
      ),
      http.post(
        "*/v1/buddy/operator/pending-publications/run/retry",
        async ({ request }) => {
          requestBody = await request.json();
          return envelope([]);
        },
      ),
    );
    render(
      <BuddyPublicationRecovery
        status={status}
        identity={buddyOperatorIdentity(store.getState())}
      />,
      { store },
    );
    const retry = await screen.findByRole("button", {
      name: "Retry saving commit_message",
    });
    expect(screen.getByText(/memory-only/)).toBeTruthy();
    await userEvent.click(retry);
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /Retry saving/ })).toBeNull(),
    );
    expect(requestBody).toEqual({
      project: status.project,
      worker: status.worker,
    });
  });
  it("rejects report bodies and duplicate metadata from the recovery transport", () => {
    const row = {
      run_id: "run",
      workflow_id: "commit_message",
      started_at: "2026-09-05T00:00:00Z",
    };
    expect(parsePendingPublications([row])).toEqual([row]);
    expect(() =>
      parsePendingPublications([{ ...row, body: "private" }]),
    ).toThrow();
    expect(() => parsePendingPublications([row, row])).toThrow();
  });
});
