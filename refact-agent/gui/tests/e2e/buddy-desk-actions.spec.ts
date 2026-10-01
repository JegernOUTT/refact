import { expect, test } from "@playwright/test";

const project = { project_id: "scratch", canonical_root: "/scratch/operator" };
const record = (value: unknown) => ({ value, revision: 1, change_seq: 1 });
const envelope = (data: unknown) => ({ schema_version: 1, project, data });
const work = {
  schema_version: 1,
  id: "work",
  project,
  subject: {
    resource_kind: "workflow",
    resource_id: "provider-check",
    version: "1",
  },
  kind: "assessment",
  title: "Provider assessment",
  state: "needs_attention",
  native_sources: [],
  current_artifact_id: "report",
  current_artifact_revision: 1,
  last_material_change_at: "2026-09-05T00:00:00Z",
};
const artifact = {
  schema_version: 1,
  artifact_id: "report",
  revision: 1,
  project,
  source_run_ids: ["run"],
  title: "Saved assessment",
  conclusion: "Assessment available, not verified.",
  delta: "",
  body: "Read the **saved findings** here.\n\n[Untrusted link](https://example.invalid/collect)\n\n![Remote image](https://example.invalid/image)",
  uncertainty: "No independent finding verification.",
  proposed_action_ids: ["unregistered-proposal"],
  evidence: [],
  provenance: {
    source_run_ids: ["run"],
    evidence_refs: [],
    contains_untrusted_source: true,
  },
};
const spec = {
  schema_version: 1,
  action_id: "check_saved_evidence",
  kind: "check_saved_evidence",
  adapter: "operator.saved_evidence",
  project,
  resource: {
    resource_kind: "saved_report",
    resource_id: "work",
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
};

for (const width of [240, 360, 768, 1280]) {
  test(`connected Desk saved-action receipt at ${width}px`, async ({
    page,
    baseURL,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const origin = new URL(baseURL ?? "http://127.0.0.1:4173").origin;
    let receipt: unknown = null;
    let readState: unknown = null;
    let snoozeState: unknown = null;
    let answerState = "pending";
    let answers = 0;
    const question = () => ({
      project_root: project.canonical_root,
      chat_id: "chat-a",
      question: {
        tool_call_id: "ask-a",
        asked_at_ms: 1,
        status: answerState,
        questions: [
          { id: "continue", type: "yes_no", text: "Continue this check?" },
        ],
      },
    });
    let executions = 0;
    const requests: string[] = [];
    await page.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (url.pathname.includes("/v1/"))
        requests.push(`${request.method()} ${url.href}`);
      if (url.origin !== origin) return route.abort();
      if (!url.pathname.startsWith("/v1/")) return route.continue();
      if (url.pathname === "/v1/buddy/work")
        return route.fulfill({ json: envelope([record(work)]) });
      if (url.pathname === "/v1/buddy/work/actions")
        return route.fulfill({
          json: envelope([
            {
              work_id: "work",
              actions: [
                {
                  spec,
                  spec_hash: "hash",
                  work_revision: 1,
                  artifact_id: "report",
                  artifact_revision: 1,
                  label: "Check saved evidence links",
                  description:
                    "Checks stored metadata only; no findings are verified and no fix is applied.",
                  latest_execution: receipt,
                },
              ],
              error: null,
            },
          ]),
        });
      if (url.pathname === "/v1/buddy/artifacts/report")
        return route.fulfill({ json: envelope(record(artifact)) });
      if (url.pathname === "/v1/buddy/artifacts/report/revisions")
        return route.fulfill({
          json: envelope([record({ ...artifact, title: "Prior report" })]),
        });
      if (url.pathname === "/v1/buddy/work/work/snooze") {
        if (request.method() === "POST")
          snoozeState = {
            schema_version: 1,
            project,
            work_id: "work",
            artifact_id: "report",
            artifact_revision: 1,
            until: "2099-01-01T00:00:00Z",
          };
        return route.fulfill({ json: envelope(snoozeState) });
      }
      if (url.pathname === "/v1/buddy/work/work/read") {
        if (request.method() === "POST") {
          expect(request.postDataJSON()).toEqual({
            project_id: "scratch",
            artifact_id: "report",
            artifact_revision: 1,
          });
          readState = {
            schema_version: 1,
            project,
            work_id: "work",
            artifact_id: "report",
            last_read_revision: 1,
            read_at: "2026-09-05T00:00:01Z",
            snoozed_until: null,
            muted: false,
          };
        }
        return route.fulfill({ json: envelope(readState) });
      }
      if (url.pathname === "/v1/buddy/decisions/current")
        return route.fulfill({
          json: {
            schema_version: 1,
            project_root: project.canonical_root,
            coverage: "loaded_native_sessions",
            truncated: false,
            questions: answerState === "pending" ? [question()] : [],
          },
        });
      if (url.pathname === "/v1/buddy/decisions/questions/chat-a/ask-a") {
        if (request.method() === "POST") {
          const body = request.postDataJSON() as Record<string, unknown>;
          expect(body.project_root).toBe(project.canonical_root);
          expect(body.asked_at_ms).toBe(1);
          expect(body.answers).toEqual([{ id: "continue", answer: false }]);
          answers += 1;
          return route.fulfill({ status: 202, json: { status: "accepted" } });
        }
        return route.fulfill({ json: question() });
      }
      if (url.pathname === "/v1/buddy/work/work/actions")
        return route.fulfill({
          json: envelope([
            {
              spec,
              spec_hash: "hash",
              work_revision: 1,
              artifact_id: "report",
              artifact_revision: 1,
              label: "Check saved evidence links",
              description:
                "Checks stored metadata only; no findings are verified and no fix is applied.",
              latest_execution: receipt,
            },
          ]),
        });
      if (
        url.pathname ===
          "/v1/buddy/work/work/actions/check_saved_evidence/execute" &&
        request.method() === "POST"
      ) {
        const body = JSON.parse(request.postData() ?? "null") as Record<
          string,
          unknown
        >;
        expect(body.project_id).toBe("scratch");
        expect(body.spec_hash).toBe("hash");
        expect(Object.keys(body).sort()).toEqual(
          [
            "project_id",
            "work_revision",
            "artifact_id",
            "artifact_revision",
            "spec_hash",
            "idempotency_key",
          ].sort(),
        );
        executions += 1;
        receipt = record({
          schema_version: 1,
          execution_id: "receipt",
          project,
          worker: {
            worker_id: "worker",
            engine_instance_id: "engine",
            epoch: 1,
          },
          idempotency_key: body.idempotency_key,
          action_id: spec.action_id,
          spec_hash: "hash",
          requester: {
            source_kind: "check",
            source_id: "desk",
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
        return route.fulfill({ json: envelope(receipt) });
      }
      return route.fulfill({
        status: 503,
        json: { detail: "Not part of isolated fixture" },
      });
    });
    await page.goto("/tests/e2e/buddy-operator-preview.html?connected=1");
    await page.getByRole("radio", { name: "No", exact: true }).check();
    await page.getByRole("button", { name: "Submit answer" }).click();
    await expect(page.getByText(/Answer queued/)).toBeVisible();
    await expect(page.getByText(/Answered — confirmed/)).toHaveCount(0);
    answerState = "answered";
    await page.getByRole("button", { name: "Refresh decision status" }).click();
    await expect(page.getByText(/Answered — confirmed/)).toBeVisible();
    expect(answers).toBe(1);
    const row = page.getByRole("button", { name: /Provider assessment/ });
    await row.click();
    await expect(
      page.getByRole("heading", { name: "Saved assessment" }),
      requests.join("\n"),
    ).toBeVisible();
    await page.getByRole("button", { name: "Mark this revision read" }).click();
    await expect(
      page.getByText("This report revision is marked read."),
    ).toBeVisible();
    await expect(
      page.getByRole("link", { name: "Untrusted link" }),
    ).toHaveCount(0);
    await expect(page.locator('img[src*="example.invalid"]')).toHaveCount(0);
    await page.getByRole("button", { name: "Snooze for one hour" }).click();
    await expect(page.getByText(/Snoozed until/)).toBeVisible();
    await page.getByText("Saved report revisions", { exact: true }).click();
    await expect(page.getByText("Revision 1 · Prior report")).toBeVisible();
    await page
      .getByRole("button", { name: "Check saved evidence links" })
      .click();
    await expect(page.getByText("Saved evidence check: Partial")).toBeVisible();
    await expect(
      page.getByText("Findings are not independently verified."),
    ).toBeVisible();
    await page.getByRole("button", { name: "Back to work list" }).click();
    await expect(row).toBeFocused();
    await row.click();
    await expect(page.getByText("Saved evidence check: Partial")).toBeVisible();
    await page.reload();
    await row.click();
    await expect(page.getByText("Saved evidence check: Partial")).toBeVisible();
    expect(executions).toBe(1);
    await expect(
      page.getByText("This report revision is marked read."),
    ).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
  });
}
