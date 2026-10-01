import { expect, test } from "@playwright/test";

const project = { project_id: "showcase", canonical_root: "/workspace/refact" };
const record = (value: unknown) => ({ value, revision: 1, change_seq: 1 });
const envelope = (data: unknown) => ({
  schema_version: 1,
  project,
  data,
  next_cursor: null,
});
const work = {
  schema_version: 1,
  id: "refresh-work",
  project,
  subject: {
    resource_kind: "workflow",
    resource_id: "buddy_security_whisperer",
    version: "1",
  },
  kind: "assessment",
  title: "Security assessment",
  state: "needs_attention",
  native_sources: [],
  current_artifact_id: "refresh-report",
  current_artifact_revision: 1,
  last_material_change_at: "2026-09-05T00:00:00Z",
};
const artifact = {
  schema_version: 1,
  artifact_id: "refresh-report",
  revision: 1,
  project,
  source_run_ids: ["run"],
  title: "Saved security assessment",
  conclusion: "Saved findings remain available.",
  delta: "",
  body: "Read the saved assessment.",
  uncertainty: null,
  proposed_action_ids: [],
  evidence: [],
  provenance: {
    source_run_ids: ["run"],
    evidence_refs: [],
    contains_untrusted_source: false,
  },
};

test("refresh keeps an open report and safely closes it when it disappears", async ({
  page,
  baseURL,
}) => {
  const origin = new URL(baseURL ?? "http://127.0.0.1:4173").origin;
  let listRequests = 0;
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== origin) return route.abort();
    if (!url.pathname.startsWith("/v1/")) return route.continue();
    if (url.pathname === "/v1/buddy/work") {
      listRequests += 1;
      return route.fulfill({
        json: envelope(listRequests < 3 ? [record(work)] : []),
      });
    }
    if (url.pathname === "/v1/buddy/artifacts/refresh-report")
      return route.fulfill({ json: envelope(record(artifact)) });
    return route.fulfill({ status: 503, json: { detail: "fixture only" } });
  });

  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(
    "/tests/e2e/route-showcase.html?route=buddy&operator=enabled",
  );
  const row = page.getByRole("button", { name: /Security assessment/ });
  await row.click();
  const reportHeading = page.getByRole("heading", {
    name: "Saved security assessment",
  });
  await expect(reportHeading).toBeVisible();

  const refresh = page.getByRole("button", { name: "Refresh work" });
  await refresh.click();
  await expect(reportHeading).toBeVisible();
  await expect(page.getByLabel("Work detail")).toBeVisible();
  await expect(row).toBeVisible();

  await row.click();
  await refresh.click();
  await expect(
    page.getByRole("status").filter({
      hasText: "That item is no longer in this view.",
    }),
  ).toBeVisible();
  await expect(page.getByLabel("Work detail")).toHaveCount(0);
  await expect(refresh).toBeFocused();
  expect(
    await page.evaluate(() => document.activeElement === document.body),
  ).toBe(false);
});
