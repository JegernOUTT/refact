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
  id: "snooze-work",
  project,
  subject: {
    resource_kind: "workflow",
    resource_id: "buddy_error_detective",
    version: "1",
  },
  kind: "assessment",
  title: "Error Detective assessment",
  state: "needs_attention",
  native_sources: [],
  current_artifact_id: "snooze-report",
  current_artifact_revision: 1,
  last_material_change_at: "2026-09-05T00:00:00Z",
};
const artifact = {
  schema_version: 1,
  artifact_id: "snooze-report",
  revision: 1,
  project,
  source_run_ids: ["run"],
  title: "Saved Error Detective assessment",
  conclusion: "The retained report is available.",
  delta: "",
  body: "Read the retained assessment.",
  uncertainty: null,
  proposed_action_ids: [],
  evidence: [],
  provenance: {
    source_run_ids: ["run"],
    evidence_refs: [],
    contains_untrusted_source: false,
  },
};

for (const width of [360, 1280]) {
  test(`a snoozed report leaves the active list until the snooze ends at ${width}px`, async ({
    page,
    baseURL,
  }) => {
    const origin = new URL(baseURL ?? "http://127.0.0.1:4173").origin;
    let snoozed = false;
    await page.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      if (url.origin !== origin) return route.abort();
      if (!url.pathname.startsWith("/v1/")) return route.continue();
      if (url.pathname === "/v1/buddy/work") {
        const attention = url.searchParams.get("attention");
        const visible =
          attention === "snoozed"
            ? snoozed
            : attention === "all"
              ? true
              : !snoozed;
        return route.fulfill({ json: envelope(visible ? [record(work)] : []) });
      }
      if (url.pathname === "/v1/buddy/artifacts/snooze-report")
        return route.fulfill({ json: envelope(record(artifact)) });
      if (url.pathname === "/v1/buddy/work/snooze-work/snooze") {
        if (route.request().method() === "POST") {
          snoozed = (await route.request().postDataJSON()).minutes !== null;
        }
        return route.fulfill({
          json: envelope(
            snoozed
              ? {
                  schema_version: 1,
                  project,
                  work_id: work.id,
                  artifact_id: artifact.artifact_id,
                  artifact_revision: artifact.revision,
                  until: "2099-01-01T00:00:00Z",
                }
              : null,
          ),
        });
      }
      return route.fulfill({ status: 503, json: { detail: "fixture only" } });
    });

    await page.setViewportSize({ width, height: 900 });
    await page.goto(
      "/tests/e2e/route-showcase.html?route=buddy&operator=enabled",
      { waitUntil: "domcontentloaded" },
    );
    await page.getByRole("tab", { name: "Reports", exact: true }).click();
    const row = page.getByRole("button", {
      name: /Error Detective assessment/,
    });
    await expect(row).toBeVisible();
    await row.click();
    const detail = page.getByLabel("Work detail");
    const snooze = detail.getByRole("region", { name: "Work snooze" });
    await expect(
      page.getByRole("heading", { name: "Saved Error Detective assessment" }),
    ).toBeVisible();
    const snoozeButton = snooze.getByRole("button", {
      name: "Snooze for one hour",
    });
    await expect(snoozeButton).toBeEnabled();
    await snoozeButton.click();
    await expect(snooze.getByText(/Snoozed until/)).toBeVisible();
    await page.getByRole("button", { name: "Back to work list" }).click();
    await expect(row).toHaveCount(0);
    await page.getByRole("button", { name: "Browse saved work" }).click();
    const attention = page.getByRole("combobox", { name: "Report attention" });
    await expect(attention).toHaveValue("unsnoozed");
    await attention.selectOption("snoozed");
    await expect(row).toBeVisible();
    await page.reload({ waitUntil: "domcontentloaded" });
    await page.getByRole("tab", { name: "Reports", exact: true }).click();
    await page.getByRole("button", { name: "Browse saved work" }).click();
    await expect(
      page.getByRole("combobox", { name: "Report attention" }),
    ).toHaveValue("unsnoozed");
    await expect(row).toHaveCount(0);
    const snoozedWork = page.waitForResponse((response) => {
      const url = new URL(response.url());
      return (
        url.pathname === "/v1/buddy/work" &&
        url.searchParams.get("attention") === "snoozed"
      );
    });
    await page
      .getByRole("combobox", { name: "Report attention" })
      .selectOption("snoozed");
    await snoozedWork;
    await expect(row).toBeVisible();
  });
}
