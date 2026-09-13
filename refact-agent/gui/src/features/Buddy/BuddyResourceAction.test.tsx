import { configureStore } from "@reduxjs/toolkit";
import { Provider } from "react-redux";
import { Theme } from "@radix-ui/themes";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { buddyApi } from "../../services/refact/buddy";
import { buddyOperatorIdentity } from "../../services/refact/buddyOperator";
import { BuddyResourceAction } from "./BuddyResourceAction";
import type { ActionExecution, ActionSpec } from "./operatorTypes";
const config = { lspPort: 8001, apiKey: "fixture" };
const spec: ActionSpec = {
  schema_version: 1,
  action_id: "apply",
  kind: "hook_project_apply",
  adapter: "hooks",
  project: { project_id: "p", canonical_root: "/repo" },
  resource: {
    resource_kind: "hook",
    resource_id: "project-hook",
    version: "v1",
  },
  expected_version: "v1",
  parameter_ref: "draft",
  availability: "available",
  authority_class: "mutating",
  confirmation: "user_confirmation_required",
  capabilities: {
    can_preview: true,
    can_execute: true,
    can_verify: true,
    can_recover: true,
    can_cancel: false,
  },
};
const rights = ["hook", { command: "fixture-only", event: "manual" }];
const envelope = (data: unknown) => ({
  schema_version: 1,
  project: spec.project,
  data,
});
const url = "*/v1/buddy";
let mutations = 0;
let grants: unknown[] = [];
let saved: ActionExecution | null = null;
const server = setupServer();
beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => {
  cleanup();
  server.resetHandlers();
  localStorage.clear();
  mutations = 0;
  grants = [];
  saved = null;
});
afterAll(() => server.close());
function mount(action = spec) {
  const store = configureStore({
    reducer: { config: () => config, [buddyApi.reducerPath]: buddyApi.reducer },
    middleware: (gdm) => gdm().concat(buddyApi.middleware),
  });
  return render(
    <Provider store={store}>
      <Theme>
        <BuddyResourceAction
          spec={action}
          endpointIdentity={buddyOperatorIdentity({ config })}
          label="Registered fixture"
        />
      </Theme>
    </Provider>,
  );
}
function handlers(
  outcome: "partial" | "outcome_unknown" | "duplicate" | "lost" = "partial",
) {
  server.use(
    http.post(`${url}/actions/preview`, () =>
      HttpResponse.json(
        envelope({
          resource_id: spec.resource.resource_id,
          expected_version: "v1",
          proposed_version: "v2",
          reload_needed: true,
          diff: "-before\n+after",
        }),
      ),
    ),
    http.post(`${url}/actions/rights`, () =>
      HttpResponse.json(envelope(rights)),
    ),
    http.post(`${url}/actions/prepare`, () =>
      HttpResponse.json(
        envelope({ spec, spec_hash: "hash", bound_parameter_ref: "draft" }),
      ),
    ),
    http.post(`${url}/actions/grant`, async ({ request }) => {
      const body = (await request.json()) as {
        spec_hash: string;
        native_rights: unknown;
        recovery_execution_id: string | null;
      };
      grants.push(body);
      return HttpResponse.json(
        envelope({
          spec_hash: body.spec_hash,
          native_rights: body.native_rights,
          recovery_execution_id: body.recovery_execution_id,
          requester: {
            source_kind: "external",
            source_id: "authenticated-user",
            source_version: "operator-http-v1",
          },
          decision: {
            project: spec.project,
            worker: { worker_id: "w", engine_instance_id: "e", epoch: 1 },
            native: {
              source_kind: "external",
              source_id: "grant",
              source_version: "resource-grant-v1",
            },
          },
          expires_at: "2999-01-01T00:00:00Z",
          revoked: false,
        }),
      );
    }),
    http.post(`${url}/actions/execute`, async ({ request }) => {
      mutations++;
      const body = (await request.json()) as {
        execution_id: string;
        idempotency_key: string;
      };
      saved = {
        schema_version: 1,
        execution_id: body.execution_id,
        idempotency_key: body.idempotency_key,
        project: spec.project,
        worker: { worker_id: "w", engine_instance_id: "e", epoch: 1 },
        action_id: spec.action_id,
        spec_hash: "hash",
        requester: {
          source_kind: "external",
          source_id: "u",
          source_version: "1",
        },
        decision: null,
        status:
          outcome === "outcome_unknown" || outcome === "lost"
            ? "outcome_unknown"
            : "partial",
        started_at: null,
        ended_at: null,
        effects: [],
        verification_evidence: ["Disk done; reload needed"],
        recovery: {
          recovery_id: "r",
          action_id: spec.action_id,
          expected_post_version: "v2",
          available: true,
        },
      };
      if (outcome === "lost") return HttpResponse.error();
      return HttpResponse.json(
        envelope({
          [outcome === "duplicate" ? "Duplicate" : "Finished"]: {
            execution: saved,
            effects: saved.effects,
            verification_evidence: saved.verification_evidence,
            recovery: saved.recovery,
          },
        }),
      );
    }),
    http.get(`${url}/executions/:id`, () =>
      HttpResponse.json(envelope({ value: saved, revision: 1, change_seq: 1 })),
    ),
  );
}
async function consent() {
  fireEvent.click(screen.getByRole("button", { name: "Preview exact change" }));
  const confirm = await screen.findByLabelText(
    "I confirm this exact scope, version, and change.",
  );
  expect(screen.getByText(/before/)).toBeDefined();
  expect(
    screen.getByRole("button", { name: "Grant and execute once" }),
  ).toBeDisabled();
  fireEvent.click(confirm);
  expect(
    screen.getByRole("button", { name: "Grant and execute once" }),
  ).toBeDisabled();
  fireEvent.click(
    screen.getByLabelText(
      "I authorize exactly these native command, tools, or install rights.",
    ),
  );
}
describe("registered resource action with real RTK and MSW", () => {
  test.each(["partial", "outcome_unknown", "duplicate"] as const)(
    "explicit consent preserves %s and prevents duplicate clicks",
    async (outcome) => {
      handlers(outcome);
      mount();
      expect(grants).toHaveLength(0);
      await consent();
      const apply = screen.getByRole("button", {
        name: "Grant and execute once",
      });
      fireEvent.click(apply);
      fireEvent.click(apply);
      await screen.findByText(/Observed result:/);
      expect(mutations).toBe(1);
      expect(grants).toHaveLength(1);
      expect(apply).toBeDisabled();
      expect(
        screen.getByText(
          outcome === "outcome_unknown"
            ? /Observed result: Outcome unknown/
            : /Observed result: Partial/,
        ),
      ).toBeDefined();
      if (outcome === "duplicate")
        expect(screen.getByText(/Existing execution returned/)).toBeDefined();
    },
  );
  test("lost response survives remount and refresh does not replay", async () => {
    handlers("lost");
    const view = mount();
    await consent();
    fireEvent.click(
      screen.getByRole("button", { name: "Grant and execute once" }),
    );
    await screen.findByRole("alert");
    expect(mutations).toBe(1);
    view.unmount();
    mount();
    expect(
      screen.getByRole("button", { name: "Preview exact change" }),
    ).toBeDisabled();
    fireEvent.click(
      screen.getByRole("button", { name: "Refresh saved execution" }),
    );
    await screen.findByText(/Observed result: Outcome unknown/);
    expect(mutations).toBe(1);
  });
  test("stale registered specs remain disabled without any request", () => {
    mount({ ...spec, availability: "stale" });
    expect(
      screen.getByRole("button", { name: "Preview exact change" }),
    ).toBeDisabled();
    expect(grants).toHaveLength(0);
  });
  test("permission denial cannot execute", async () => {
    handlers();
    server.use(
      http.post(
        `${url}/actions/grant`,
        () => new HttpResponse(null, { status: 403 }),
      ),
    );
    mount();
    await consent();
    fireEvent.click(
      screen.getByRole("button", { name: "Grant and execute once" }),
    );
    await screen.findByRole("alert");
    expect(mutations).toBe(0);
  });
  test("recovery mints a separate grant for the original execution only", async () => {
    handlers();
    server.use(
      http.post(
        `${url}/executions/:id/recover`,
        async ({ params, request }) => {
          const body = (await request.json()) as {
            execution_id: string;
            idempotency_key: string;
          };
          if (!saved) throw new Error("missing fixture original");
          const restored = {
            ...saved,
            execution_id: body.execution_id,
            idempotency_key: body.idempotency_key,
            action_id: `recovery:${String(params.id)}:r`,
            spec_hash: "recovery-hash",
            status: "completed",
            recovery: null,
          };
          return HttpResponse.json(
            envelope({
              execution: restored,
              effects: restored.effects,
              verification_evidence: restored.verification_evidence,
              recovery: null,
            }),
          );
        },
      ),
    );
    mount();
    await consent();
    fireEvent.click(
      screen.getByRole("button", { name: "Grant and execute once" }),
    );
    await screen.findByText(/Observed result:/);
    const recover = screen.getByRole("button", {
      name: "Grant original-execution recovery",
    });
    expect(recover).toBeDisabled();
    fireEvent.click(
      screen.getByLabelText(
        "I authorize recovery of this original execution only.",
      ),
    );
    fireEvent.click(recover);
    await waitFor(() => expect(grants).toHaveLength(2));
    expect(grants[1]).toMatchObject({
      recovery_execution_id: saved?.execution_id,
      native_rights: null,
    });
    await screen.findByText(/Observed result: Completed/);
    expect(mutations).toBe(1);
  });
});
