// Harness choice: this gate runs against a minimal Vite-served route showcase
// instead of Storybook or the live app entrypoint. It renders real dashboard and
// chat React surfaces with mocked Redux state, so CI does not need a running LSP.
import { expect, test } from "@playwright/test";

// Buddy has exactly five top-level sections. Legacy Pulse/Opportunities live
// inside Reports and legacy Recent errors inside Activity (operator disabled),
// so each panel is checked inside the section that owns it.
const BUDDY_TOP_SECTIONS = [
  "Home",
  "Reports",
  "Activity",
  "Play",
  "Settings",
] as const;

const BUDDY_TAB_PANELS: {
  tab: (typeof BUDDY_TOP_SECTIONS)[number];
  selector: string;
  description: string;
}[] = [
  {
    tab: "Home",
    selector: "[data-testid='buddy-home-overview']",
    description: "Buddy Home overview",
  },
  {
    tab: "Home",
    selector: "[data-testid='buddy-home-hero']",
    description: "Buddy Home modest scene",
  },
  {
    tab: "Reports",
    selector: "[data-testid='buddy-pulse-card']",
    description: "Buddy pulse card (legacy, inside Reports)",
  },
  {
    tab: "Reports",
    selector: "[data-testid='buddy-opportunities-feed']",
    description: "Buddy opportunities feed (legacy, inside Reports)",
  },
  {
    tab: "Activity",
    selector: "[data-testid='buddy-recent-errors-panel']",
    description: "Buddy recent errors panel (legacy, inside Activity)",
  },
  {
    tab: "Activity",
    selector: "[data-testid='buddy-activity-panel']",
    description: "Buddy activity panel",
  },
  {
    tab: "Play",
    selector: "[data-testid='buddy-home-hero']",
    description: "Buddy Play scene",
  },
  {
    tab: "Play",
    selector: "[data-testid='buddy-summary-strip']",
    description: "Buddy summary strip (Play)",
  },
  {
    tab: "Settings",
    selector: "[data-testid='buddy-personality-panel']",
    description: "Buddy personality panel",
  },
  {
    tab: "Settings",
    selector: "[data-testid='buddy-settings-panel']",
    description: "Buddy settings panel",
  },
];

const widths = [240, 360, 768, 1280] as const;

const SCENE_SECTIONS: readonly string[] = ["Home", "Play"];

test.describe("Buddy sections", () => {
  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  for (const width of widths) {
    test(`buddy sections select, mount one scene and take focus at ${width}px`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto("/tests/e2e/route-showcase.html?route=buddy");
      await page.locator("[data-element='app-root']").waitFor();
      await page.getByTestId("buddy-home-content").waitFor();
      const sections = page
        .getByRole("tablist", { name: "Buddy sections" })
        .getByRole("tab");
      await expect(sections).toHaveText([...BUDDY_TOP_SECTIONS]);
      await expect(
        page.getByRole("tab", { name: "Home", exact: true }),
      ).toHaveAttribute("aria-selected", "true");
      await expect(page.getByTestId("buddy-home-overview")).toBeVisible();
      await expect(page.getByTestId("buddy-presence")).toBeVisible();
      await expect(page.getByTestId("buddy-world")).toBeVisible();
      await expect(page.getByTestId("buddy-home-hero")).toBeVisible();

      for (const panel of BUDDY_TAB_PANELS) {
        const tab = page.getByRole("tab", { name: panel.tab, exact: true });
        await tab.click();
        await expect(tab).toHaveAttribute("aria-selected", "true");
        await expect(
          page.locator(panel.selector),
          panel.description,
        ).toBeVisible();
        // Only the active section's BuddyWorld may be mounted, and the scene
        // sections must actually mount it.
        const scene = SCENE_SECTIONS.includes(panel.tab);
        await expect(
          page.getByTestId("buddy-world"),
          `${panel.tab}: mounted BuddyWorld count`,
        ).toHaveCount(scene ? 1 : 0);
        if (scene) await expect(page.getByTestId("buddy-world")).toBeVisible();
      }

      for (const name of BUDDY_TOP_SECTIONS) {
        const tab = page.getByRole("tab", { name, exact: true });
        await tab.focus();
        await expect(tab).toBeFocused();
      }
    });
  }
});

test.describe("overlay keyboard regressions", () => {
  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  test("Escape dismisses compression and model overlays and restores trigger focus", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto("/tests/e2e/route-showcase.html?route=overlay-regression");
    await page.locator("[data-element='app-root']").waitFor();

    const trajectoryTrigger = page.getByLabel("Compress or rebuild context");
    const compressionTab = page.getByRole("tab", {
      name: "LLM summary",
      exact: true,
    });
    const trajectoryPopover = page.getByRole("dialog").filter({
      has: compressionTab,
    });
    await trajectoryTrigger.click();
    await trajectoryPopover.waitFor();

    await compressionTab.click();
    await expect(compressionTab).toHaveAttribute("aria-selected", "true");

    await compressionTab.focus();
    await expect(compressionTab).toBeFocused();
    await compressionTab.press("Escape");
    await expect(trajectoryPopover).toHaveCount(0);
    await expect(trajectoryTrigger).toHaveAttribute("aria-expanded", "false");
    await expect(trajectoryTrigger).toBeFocused();
    await page.mouse.move(0, 0);
    await trajectoryTrigger.blur();
    const modelTrigger = page.getByRole("button", {
      name: "Select model",
      exact: true,
    });
    const models = page.getByRole("listbox", { name: "Models", exact: true });
    const modelPopover = page.getByRole("dialog").filter({ has: models });
    await modelTrigger.click();
    await models.waitFor();

    const modelSearch = modelPopover.getByRole("searchbox");
    await modelSearch.focus();
    await expect(modelSearch).toBeFocused();
    await modelSearch.press("Escape");
    await expect(models).toHaveCount(0);
    await expect(modelPopover).toHaveCount(0);
    await expect(modelTrigger).toHaveAttribute("aria-expanded", "false");
    await expect(modelTrigger).toBeFocused();
    await page.mouse.move(0, 0);
    const modeTrigger = page.getByRole("button", {
      name: "Mode: Agent",
      exact: true,
    });
    await modeTrigger.click();
    const modePopover = page.getByRole("dialog");
    await expect(modePopover).toBeVisible();
    const activeMode = modePopover.getByRole("button", {
      name: "Agent restart Autonomous coding mode editing · tools 12 tools",
      exact: true,
    });
    await expect(activeMode).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(modePopover).toHaveCount(0);
    await expect(modeTrigger).toHaveAttribute("aria-expanded", "false");
    await expect(modeTrigger).toBeFocused();
  });
});

test.describe("Buddy route smoke", () => {
  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  test("navigates the five Buddy sections without clipping", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 360, height: 900 });
    await page.goto("/tests/e2e/route-showcase.html?route=buddy");
    await page.getByTestId("buddy-home-content").waitFor();

    // Home is the default: modest real scene plus presence, no summary strip
    // (that moved to Play) and no legacy Pulse/Opportunities/Recent errors tabs.
    await expect(
      page.getByRole("tab", { name: "Home", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("buddy-home-overview")).toBeVisible();
    await expect(page.getByTestId("buddy-home-hero")).toBeVisible();
    await expect(page.getByTestId("buddy-world")).toHaveCount(1);
    await expect(page.getByTestId("buddy-presence-avatar")).toHaveCount(0);
    await expect(page.getByTestId("buddy-summary-strip")).toHaveCount(0);
    for (const legacyTab of ["Pulse", "Opportunities", "Recent errors"]) {
      await expect(
        page.getByRole("tab", { name: legacyTab, exact: true }),
      ).toHaveCount(0);
    }
    await expect(page.getByTestId("buddy-home-saved-work")).toBeVisible();

    // Open Play switches to the full scene section, mounted immediately with
    // no extra open/close toggle; Leave Play returns to Home.
    await page.getByRole("button", { name: "Open Play", exact: true }).click();
    await expect(
      page.getByRole("tab", { name: "Play", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("buddy-home-play")).toBeVisible();
    await expect(page.getByTestId("buddy-world")).toHaveCount(1);
    await expect(page.getByTestId("buddy-world")).toBeInViewport();
    await expect(page.getByTestId("buddy-summary-strip")).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Close Play", exact: true }),
    ).toHaveCount(0);
    await page.getByRole("tab", { name: "Home", exact: true }).click();
    await expect(
      page.getByRole("tab", { name: "Home", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("buddy-home-overview")).toBeVisible();

    // Legacy Pulse and Opportunities are stacked inside Reports.
    await page.getByRole("tab", { name: "Reports", exact: true }).click();
    await expect(page.getByTestId("buddy-home-reports")).toBeVisible();
    await expect(page.getByTestId("buddy-pulse-card")).toBeVisible();
    await expect(page.getByTestId("buddy-opportunities-feed")).toBeVisible();
    await expect(page.getByTestId("buddy-world")).toHaveCount(0);

    // Legacy Recent errors folds into Activity above the activity log.
    await page.getByRole("tab", { name: "Activity", exact: true }).click();
    await expect(page.getByTestId("buddy-recent-errors-panel")).toBeVisible();
    await expect(page.getByTestId("buddy-activity-panel")).toBeVisible();

    await page
      .getByRole("radio", { name: "Buddy workflows", exact: true })
      .check({ force: true });
    await expect(
      page.getByText("Buddy reviewed responsive surfaces"),
    ).toBeVisible();
    await expect(page.getByText("Refact e2e gate queued")).not.toBeVisible();

    const measureClipped = (selectors: string[]) =>
      page.evaluate<string[], string[]>((entries) => {
        const viewportWidth = document.documentElement.clientWidth;
        return entries.flatMap((selector) => {
          const el = document.querySelector(selector);
          if (!el) return [`${selector} missing`];
          const rect = el.getBoundingClientRect();
          const isClipped = rect.left < -1 || rect.right > viewportWidth + 1;
          return isClipped
            ? [
                `${selector} rect=${rect.left},${rect.top},${rect.right},${rect.bottom}`,
              ]
            : [];
        });
      }, selectors);

    const activityClipped = await measureClipped([
      "[data-testid='buddy-activity-panel']",
    ]);
    expect(
      activityClipped,
      `horizontally clipped Buddy panels: ${activityClipped.join(" | ")}`,
    ).toEqual([]);

    await page.getByRole("tab", { name: "Settings", exact: true }).click();
    await expect(page.getByTestId("buddy-settings-panel")).toBeVisible();

    const settingsClipped = await measureClipped([
      "[data-testid='buddy-settings-panel']",
    ]);
    expect(
      settingsClipped,
      `horizontally clipped Buddy panels: ${settingsClipped.join(" | ")}`,
    ).toEqual([]);
  });
});
