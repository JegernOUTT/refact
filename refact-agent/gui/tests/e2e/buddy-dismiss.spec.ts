import { expect, test } from "@playwright/test";

const project = { project_id: "showcase", canonical_root: "/workspace/refact" };
const record = (value: unknown) => ({ value, revision: 1, change_seq: 1 });
const envelope = (data: unknown) => ({
  schema_version: 1,
  project,
  data,
  next_cursor: null,
});
const titles = ["Earlier finding", "Dismissed finding", "Following finding"];
const rows = titles.map((title, index) => ({
  schema_version: 1,
  id: `dismiss-work-${index}`,
  project,
  subject: {
    resource_kind: "workflow",
    resource_id: `workflow-${index}`,
    version: "1",
  },
  kind: "assessment",
  title,
  state: "needs_attention",
  native_sources: [],
  current_artifact_id: `dismiss-artifact-${index}`,
  current_artifact_revision: 1,
  last_material_change_at: "2026-09-05T00:00:00Z",
}));
const artifacts = [0, 1, 2].map((index) => ({
  schema_version: 1,
  artifact_id: `dismiss-artifact-${index}`,
  revision: 1,
  project,
  source_run_ids: ["run"],
  title: "Dismissal report",
  conclusion: "Report available.",
  delta: "",
  body: "Saved report body.",
  uncertainty: null,
  proposed_action_ids: [],
  evidence: [],
  provenance: {
    source_run_ids: ["run"],
    evidence_refs: [],
    contains_untrusted_source: false,
  },
}));
const actionSpec = {
  schema_version: 1,
  action_id: "dismiss_not_a_problem",
  kind: "dismiss_not_a_problem",
  adapter: "operator.work_lifecycle",
  project,
  resource: {
    resource_kind: "work_item",
    resource_id: rows[1].id,
    version: "digest",
  },
  expected_version: "digest",
  parameter_ref: null,
  availability: "available",
  confirmation: "none",
  authority_class: "mutating",
  capabilities: {
    can_preview: false,
    can_execute: true,
    can_verify: true,
    can_recover: false,
    can_cancel: false,
  },
};

for (const width of [360, 1280]) {
  test(`successful dismissal is announced and focuses the next row at ${width}px`, async ({
    page,
    baseURL,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const origin = new URL(baseURL ?? "http://127.0.0.1:4173").origin;
    let activeRows = [...rows];
    await page.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (url.origin !== origin) return route.abort();
      if (!url.pathname.startsWith("/v1/")) return route.continue();
      if (url.pathname === "/v1/buddy/work")
        return route.fulfill({ json: envelope(activeRows.map(record)) });
      if (url.pathname === "/v1/buddy/work/actions") {
        const ids = url.searchParams.get("ids")?.split(",") ?? [];
        return route.fulfill({
          json: envelope(
            ids.map((work_id) => ({
              work_id,
              actions:
                work_id === "dismiss-work-1"
                  ? [
                      {
                        spec: actionSpec,
                        spec_hash: "dismiss-hash",
                        work_revision: 1,
                        artifact_id: artifacts[1].artifact_id,
                        artifact_revision: 1,
                        label: "Dismiss — not a problem",
                        description: "Moves this item out of Needs you.",
                        latest_execution: null,
                      },
                    ]
                  : [],
              error: work_id === "dismiss-work-1" ? null : "Fixture only",
            })),
          ),
        });
      }
      if (url.pathname.startsWith("/v1/buddy/artifacts/")) {
        const artifactId = url.pathname.split("/").at(-1);
        const value = artifacts.find(
          (candidate) => candidate.artifact_id === artifactId,
        );
        if (value) return route.fulfill({ json: envelope(record(value)) });
      }
      if (url.pathname === "/v1/buddy/work/dismiss-work-1/actions")
        return route.fulfill({
          json: envelope([
            {
              spec: actionSpec,
              spec_hash: "dismiss-hash",
              work_revision: 1,
              artifact_id: artifacts[1].artifact_id,
              artifact_revision: 1,
              label: "Dismiss — not a problem",
              description: "Moves this item out of Needs you.",
              latest_execution: null,
            },
          ]),
        });
      if (
        url.pathname ===
          "/v1/buddy/work/dismiss-work-1/actions/dismiss_not_a_problem/execute" &&
        request.method() === "POST"
      ) {
        const body = request.postDataJSON() as { idempotency_key: string };
        activeRows = activeRows.filter((row) => row.id !== rows[1].id);
        return route.fulfill({
          json: envelope(
            record({
              schema_version: 1,
              execution_id: "dismiss-receipt",
              project,
              worker: {
                worker_id: "worker",
                engine_instance_id: "engine",
                epoch: 1,
              },
              idempotency_key: body.idempotency_key,
              action_id: actionSpec.action_id,
              spec_hash: "dismiss-hash",
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
            }),
          ),
        });
      }
      return route.fulfill({ status: 503, json: { detail: "Fixture only" } });
    });

    await page.goto(
      "/tests/e2e/route-showcase.html?route=buddy&operator=enabled",
    );
    await page.getByRole("tab", { name: "Reports", exact: true }).click();
    await page.getByRole("button", { name: "Dismissed finding" }).click();
    await page
      .getByRole("region", { name: "Report actions" })
      .getByRole("button", { name: "Dismiss — not a problem", exact: true })
      .click();
    const status = page.getByRole("status").filter({
      hasText:
        'Dismissed "Dismissed finding". You can find it under Delivered.',
    });
    await expect(status).toBeVisible();
    await expect(status).toHaveCount(1);
    await expect(
      page.getByRole("button", { name: "Dismissed finding" }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Following finding" }),
    ).toBeFocused();
    await expect(
      page.getByText("That report is no longer in this view."),
    ).toHaveCount(0);
  });
}
