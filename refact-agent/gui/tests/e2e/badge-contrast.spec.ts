import { expect, test, type Page } from "@playwright/test";
import { measureBadgeContrast } from "./badgeContrast";

const THEMES = ["light", "dark"] as const;
const BADGE_ROUTE = "/tests/e2e/route-showcase.html?route=badges";
const BUDDY_ROUTE =
  "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const project = { project_id: "showcase", canonical_root: "/workspace/refact" };

async function openBadgeGallery(page: Page, theme: (typeof THEMES)[number]) {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(`${BADGE_ROUTE}&theme=${theme}`);
  await expect(page.getByTestId("badge-contrast").first()).toBeVisible();
}

async function openBuddyWithWorkFixture(
  page: Page,
  theme: (typeof THEMES)[number],
) {
  const work = {
    schema_version: 1,
    id: "garden-orphans",
    project,
    subject: {
      resource_kind: "memory_garden",
      resource_id: "orphan",
      version: "1",
    },
    kind: "assessment",
    title: "Knowledge base needs attention: 97 lifecycle candidate(s)",
    state: "needs_attention",
    native_sources: [],
    current_artifact_id: "artifact-garden",
    current_artifact_revision: 1,
    last_material_change_at: "2026-06-07T09:49:00Z",
  };
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/buddy/operator/status") {
      return route.fulfill({
        json: {
          schema_version: 1,
          mode: "enabled",
          features: {
            storage: true,
            actions: true,
            supervision: false,
            improvement: false,
            editorial: false,
          },
          project,
          worker: {
            worker_id: "showcase-worker",
            engine_instance_id: "showcase-engine",
            epoch: 1,
          },
        },
      });
    }

    if (url.pathname === "/v1/buddy/work") {
      return route.fulfill({
        json: {
          schema_version: 1,
          project,
          data: [{ value: work, revision: 1, change_seq: 1 }],
          next_cursor: null,
        },
      });
    }
    if (url.pathname === "/v1/buddy/work/garden-orphans") {
      return route.fulfill({
        json: {
          schema_version: 1,
          project,
          data: { value: work, revision: 1, change_seq: 1 },
          next_cursor: null,
        },
      });
    }
    if (url.pathname.startsWith("/v1/buddy/")) {
      return route.fulfill({
        status: 503,
        json: { detail: "not needed here" },
      });
    }
    return route.continue();
  });
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(`${BUDDY_ROUTE}&theme=${theme}`);
  await expect(page.getByTestId("buddy-home-content")).toBeVisible();
  await expect(page.getByTestId("buddy-operator-status")).toContainText("On");
  await expect(
    page.getByRole("button", { name: /Knowledge base needs attention/ }),
  ).toBeVisible();
}

test.describe("shared Badge text contrast", () => {
  for (const theme of THEMES) {
    test(`every tone, variant, and size meets AA in ${theme}`, async ({
      page,
    }) => {
      await openBadgeGallery(page, theme);
      const measurements = await measureBadgeContrast(
        page,
        "[data-testid='badge-contrast']",
      );
      const failures = measurements.filter(
        ({ ratio, required }) => ratio < required,
      );
      const minimums = measurements.reduce<Record<string, number>>(
        (result, measurement) => {
          const key = `${measurement.tone}/${measurement.variant}`;
          result[key] = Math.min(result[key] ?? Infinity, measurement.ratio);
          return result;
        },
        {},
      );
      console.log(`Badge ratios ${theme}: ${JSON.stringify(minimums)}`);
      expect(
        failures,
        `Badge contrast failures in ${theme}:\n${JSON.stringify(
          measurements,
          null,
          2,
        )}`,
      ).toEqual([]);
      expect(measurements).toHaveLength(54);
    });

    test(`Buddy status pill meets AA in ${theme}`, async ({ page }) => {
      await openBuddyWithWorkFixture(page, theme);
      const measurement = await measureBadgeContrast(
        page,
        "[data-testid='buddy-operator-status']",
      );
      console.log(`Buddy status ${theme}: ${JSON.stringify(measurement)}`);
      expect(
        measurement.filter(({ ratio, required }) => ratio < required),
        `Buddy status pill contrast in ${theme}:\n${JSON.stringify(
          measurement,
          null,
          2,
        )}`,
      ).toEqual([]);
      expect(measurement).toHaveLength(1);
    });
  }
});
