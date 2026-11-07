// Harness choice: this gate runs against a minimal Vite-served route showcase
// instead of Storybook or the live app entrypoint. It renders real dashboard and
// chat React surfaces with mocked Redux state, so CI does not need a running LSP.
import { expect, test } from "@playwright/test";

type OverflowReport = {
  docOverflow: boolean;
  offenders: string[];
  innerScrollOffenders: string[];
};

type BuddyInnerOverflowReport = {
  selector: string;
  description: string;
  missing: boolean;
  overflowing: boolean;
  scrollWidth: number;
  clientWidth: number;
};

type BuddyTabPanelTarget = {
  selector: string;
  description: string;
};

// Buddy has exactly five top-level sections. Legacy Pulse/Opportunities live
// inside Reports and legacy Recent errors inside Activity (operator disabled),
// so each panel is measured inside the section that owns it.
const BUDDY_TOP_SECTIONS = [
  "Home",
  "Reports",
  "Activity",
  "Play",
  "Settings",
] as const;

const BUDDY_TAB_PANELS: (BuddyTabPanelTarget & {
  tab: (typeof BUDDY_TOP_SECTIONS)[number];
})[] = [
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

type EdgeMeasurement = {
  name: string;
  delta: number;
  details: string;
};

type GutterMeasurement = {
  name: string;
  value: string;
};

const widths = [240, 360, 768, 1280] as const;

const routes = [
  {
    name: "dashboard",
    path: "/tests/e2e/route-showcase.html?route=dashboard",
  },
  {
    name: "chat",
    path: "/tests/e2e/route-showcase.html?route=chat",
  },
  {
    name: "chat split group",
    path: "/tests/e2e/route-showcase.html?route=chat-split",
  },
  {
    name: "workspace dock and terminal workbench",
    path: "/tests/e2e/route-showcase.html?route=workspace-chrome",
  },
  {
    name: "workspace disabled terminal workbench",
    path: "/tests/e2e/route-showcase.html?route=workspace-chrome-disabled",
  },
  {
    name: "buddy",
    path: "/tests/e2e/route-showcase.html?route=buddy",
  },
  {
    name: "settings general",
    path: "/tests/e2e/route-showcase.html?route=settings&settings=general",
  },
  {
    name: "settings privacy",
    path: "/tests/e2e/route-showcase.html?route=settings&settings=privacy",
  },
  {
    name: "settings providers",
    path: "/tests/e2e/route-showcase.html?route=settings&settings=providers",
  },
  {
    name: "settings models",
    path: "/tests/e2e/route-showcase.html?route=settings&settings=models",
  },
  {
    name: "settings customization",
    path: "/tests/e2e/route-showcase.html?route=settings&settings=customization",
  },
  {
    name: "settings integrations",
    path: "/tests/e2e/route-showcase.html?route=settings&settings=integrations",
  },
  {
    name: "settings scheduler",
    path: "/tests/e2e/route-showcase.html?route=settings&settings=scheduler",
  },
  {
    name: "settings documentation",
    path: "/tests/e2e/route-showcase.html?route=settings&settings=documentation",
  },
  {
    name: "settings extensions",
    path: "/tests/e2e/route-showcase.html?route=settings&settings=extensions",
  },
  {
    name: "marketplace skills",
    path: "/tests/e2e/route-showcase.html?route=marketplace&marketplace=skills",
  },
  {
    name: "marketplace commands",
    path: "/tests/e2e/route-showcase.html?route=marketplace&marketplace=commands",
  },
  {
    name: "marketplace subagents",
    path: "/tests/e2e/route-showcase.html?route=marketplace&marketplace=subagents",
  },
  {
    name: "marketplace mcp",
    path: "/tests/e2e/route-showcase.html?route=marketplace&marketplace=mcp",
  },
  {
    name: "marketplace extensions",
    path: "/tests/e2e/route-showcase.html?route=marketplace&marketplace=extensions",
  },
] as const;

test.describe("no page-level horizontal scroll", () => {
  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  for (const route of routes) {
    for (const width of widths) {
      test(`${route.name} has no page horizontal overflow at ${width}px`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.goto(route.path);
        await page.locator("[data-element='app-root']").waitFor();
        if (route.path.includes("route=settings")) {
          await page.getByRole("heading", { name: "Settings" }).waitFor();
          const mobileSectionSelect = page.locator(
            "button[aria-label='Settings sections']",
          );
          if (width <= 720) {
            await expect(mobileSectionSelect).toBeVisible();
          } else {
            await expect(mobileSectionSelect).not.toBeVisible();
          }
        }
        if (route.path.includes("route=buddy")) {
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
        }
        if (route.path.includes("route=chat-split")) {
          await page.locator("[data-workspace-group-tab-id]").waitFor();
          await expect(
            page.locator("[data-surface-key='chat:showcase-chat']"),
          ).toBeVisible();
          await expect(
            page.locator("[data-surface-key='chat:showcase-chat-b']"),
          ).toBeVisible();
        }
        if (route.path.includes("route=workspace-chrome")) {
          const terminalDisabled = route.path.includes(
            "route=workspace-chrome-disabled",
          );
          const terminalWorkbench = page.getByLabel(
            "Terminal workbench for showcase-chat",
          );
          const terminalBody = page.getByTestId("terminal-workbench-body");
          await expect(terminalWorkbench).toBeVisible();
          await expect(
            terminalWorkbench.getByText(
              terminalDisabled
                ? "Browser terminal disabled"
                : "No terminal sessions",
            ),
          ).toBeVisible();
          const readTerminalGeometry = () =>
            terminalBody.evaluate((body) => {
              const emptyState = body.querySelector("section");
              const bodyRect = body.getBoundingClientRect();
              const emptyRect = emptyState?.getBoundingClientRect();
              return {
                bodyHeight: bodyRect.height,
                bodyWidth: bodyRect.width,
                emptyHeight: emptyRect?.height ?? 0,
                emptyWidth: emptyRect?.width ?? 0,
                emptyScrollHeight: emptyState?.scrollHeight ?? 0,
                emptyClientHeight: emptyState?.clientHeight ?? 0,
              };
            });
          await expect
            .poll(async () => (await readTerminalGeometry()).bodyHeight)
            .toBeGreaterThanOrEqual(179);
          const terminalGeometry = await readTerminalGeometry();
          expect(terminalGeometry.bodyHeight).toBeGreaterThanOrEqual(179);
          expect(terminalGeometry.bodyHeight).toBeLessThanOrEqual(301);
          expect(
            Math.abs(terminalGeometry.bodyWidth - terminalGeometry.emptyWidth),
          ).toBeLessThanOrEqual(1);
          expect(
            Math.abs(
              terminalGeometry.bodyHeight - terminalGeometry.emptyHeight,
            ),
          ).toBeLessThanOrEqual(1);
          expect(terminalGeometry.emptyScrollHeight).toBeLessThanOrEqual(
            terminalGeometry.emptyClientHeight + 1,
          );
          if (width < 768) {
            await page
              .getByRole("button", { name: "Files", exact: true })
              .click();
            await expect(page.getByRole("dialog")).toBeVisible();
          } else {
            await expect(page.getByTestId("workspace-dock")).toBeVisible();
          }
        }

        const overflow = await page.evaluate<OverflowReport, boolean>(
          (checkInnerScroll) => {
            const describeElement = (el: Element) => {
              const className =
                typeof el.className === "string" ? el.className.trim() : "";
              const testId = el.getAttribute("data-testid");
              const element = el.getAttribute("data-element");
              const label = el.getAttribute("aria-label");
              const descriptor = testId ?? element ?? label ?? className;
              const size = `${el.scrollWidth}x${el.clientWidth}`;
              return descriptor
                ? `${el.tagName.toLowerCase()}.${descriptor} ${size}`
                : `${el.tagName.toLowerCase()} ${size}`;
            };
            const docOverflow =
              document.documentElement.scrollWidth >
              document.documentElement.clientWidth + 1;
            const offenders = [...document.querySelectorAll("*")]
              .filter((el) => {
                if (el.closest(".scrollX")) return false;
                const style = getComputedStyle(el);
                return (
                  el.scrollWidth > el.clientWidth + 1 &&
                  style.overflowX !== "auto" &&
                  style.overflowX !== "scroll"
                );
              })
              .slice(0, 5)
              .map(describeElement);
            const innerScrollOffenders = checkInnerScroll
              ? [...document.querySelectorAll("*")]
                  .filter((el) => {
                    if (el.closest(".scrollX")) return false;
                    if (el.getAttribute("role") === "tablist") return false;
                    const style = getComputedStyle(el);
                    return (
                      el.scrollWidth > el.clientWidth + 2 &&
                      (style.overflowX === "auto" ||
                        style.overflowX === "scroll")
                    );
                  })
                  .slice(0, 5)
                  .map(describeElement)
              : [];
            return { docOverflow, offenders, innerScrollOffenders };
          },
          route.path.includes("route=settings") ||
            route.path.includes("route=buddy"),
        );

        expect(
          overflow.docOverflow,
          `offenders: ${overflow.offenders.join(" | ")}`,
        ).toBe(false);
        expect(
          overflow.innerScrollOffenders,
          `inner scroll offenders: ${overflow.innerScrollOffenders.join(
            " | ",
          )}`,
        ).toEqual([]);

        if (route.path.includes("route=buddy")) {
          // Each target must not overflow internally AND must stay inside the
          // viewport: an overflow:hidden ancestor can still be scrolled by tab
          // focus, shifting whole sections off the left edge without any
          // document-level overflow.
          const measureBuddyTargets = (targets: BuddyTabPanelTarget[]) =>
            page.evaluate<BuddyInnerOverflowReport[], BuddyTabPanelTarget[]>(
              (entries) =>
                entries.map(({ selector, description }) => {
                  const el = document.querySelector(selector);
                  if (!el) {
                    return {
                      selector,
                      description,
                      missing: true,
                      overflowing: false,
                      scrollWidth: 0,
                      clientWidth: 0,
                    };
                  }
                  const viewportWidth = document.documentElement.clientWidth;
                  const rect = el.getBoundingClientRect();
                  const outside =
                    rect.left < -1 || rect.right > viewportWidth + 1;
                  const allowInternalScroll =
                    el.getAttribute("role") === "tablist" &&
                    el.getAttribute("aria-label") === "Buddy sections" &&
                    ["auto", "scroll"].includes(getComputedStyle(el).overflowX);
                  return {
                    selector: outside
                      ? `${selector} rect=${Math.round(
                          rect.left,
                        )}..${Math.round(rect.right)}/${viewportWidth}`
                      : selector,
                    description,
                    missing: false,
                    overflowing:
                      (!allowInternalScroll &&
                        el.scrollWidth > el.clientWidth + 2) ||
                      outside,
                    scrollWidth: el.scrollWidth,
                    clientWidth: el.clientWidth,
                  };
                }),
              targets,
            );

          const buddyInnerOverflow = await measureBuddyTargets([
            {
              selector: "[data-testid='buddy-home-content']",
              description: "Buddy content scroller",
            },
            {
              selector: "[data-testid='buddy-presence']",
              description: "Buddy compact presence",
            },
            {
              selector: "[role='tablist'][aria-label='Buddy sections']",
              description: "Buddy section tabs",
            },
          ]);

          for (const panel of BUDDY_TAB_PANELS) {
            const tab = page.getByRole("tab", { name: panel.tab, exact: true });
            await tab.click();
            await expect(tab).toHaveAttribute("aria-selected", "true");
            await page.locator(panel.selector).waitFor({ state: "visible" });
            // Only the active section's BuddyWorld may be mounted.
            expect(
              await page.getByTestId("buddy-world").count(),
              `${panel.tab}: mounted BuddyWorld count`,
            ).toBeLessThanOrEqual(1);
            buddyInnerOverflow.push(
              ...(await measureBuddyTargets([
                { selector: panel.selector, description: panel.description },
                {
                  selector: "[data-testid='buddy-home-content']",
                  description: `Buddy content scroller (${panel.tab})`,
                },
                {
                  selector: "[role='tablist'][aria-label='Buddy sections']",
                  description: `Buddy section tabs (${panel.tab})`,
                },
              ])),
            );
          }

          for (const name of BUDDY_TOP_SECTIONS) {
            const tab = page.getByRole("tab", { name, exact: true });
            await tab.focus();
            await expect(tab).toBeFocused();
            const box = await tab.boundingBox();
            expect(box, `${name} tab box`).not.toBeNull();
            expect(box!.x, `${name} tab left`).toBeGreaterThanOrEqual(-1);
            expect(
              box!.x + box!.width,
              `${name} tab right`,
            ).toBeLessThanOrEqual(width + 1);
          }

          const buddyInnerOffenders = buddyInnerOverflow.filter(
            (entry) => entry.missing || entry.overflowing,
          );
          expect(
            buddyInnerOffenders,
            buddyInnerOffenders
              .map(
                (entry) =>
                  `${entry.description} ${entry.selector} ${entry.scrollWidth}x${entry.clientWidth}`,
              )
              .join(" | "),
          ).toEqual([]);
        }
      });
    }
  }
});

test.describe("overlay right-edge regressions", () => {
  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  test("aligns overlay controls without broad scrollbar gutters", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto("/tests/e2e/route-showcase.html?route=overlay-regression");
    await page.locator("[data-element='app-root']").waitFor();

    const trajectoryTrigger = page.getByLabel("Compress or rebuild context");
    const compressionTab = page.getByRole("tab", {
      name: "LLM compression",
      exact: true,
    });
    const trajectoryPopover = page.getByRole("dialog").filter({
      has: compressionTab,
    });
    await trajectoryTrigger.click();
    await trajectoryPopover.waitFor();

    const trajectoryGutters = await page.evaluate<GutterMeasurement[]>(() => {
      const dialog = document.querySelector("[role='dialog']");
      const tab = document.querySelector("[role='tab']");
      const preview = [...document.querySelectorAll("button")].find(
        (el) => el.textContent?.trim() === "Preview",
      );
      const label = preview?.querySelector("span");
      const targets = [
        ["popover content", dialog],
        ["tabs trigger", tab],
        ["button label", label],
      ] as const;
      return targets.map(([name, el]) => ({
        name,
        value: el ? getComputedStyle(el).scrollbarGutter : "missing",
      }));
    });

    for (const gutter of trajectoryGutters) {
      expect(gutter.value, gutter.name).toBe("auto");
    }

    const trajectoryEdges = await page.evaluate<EdgeMeasurement[]>(() => {
      const rect = (el: Element) => el.getBoundingClientRect();
      const px = (value: string) => Number.parseFloat(value) || 0;
      const tablist = document.querySelector("[role='tablist']");
      const tabs = [...document.querySelectorAll("[role='tab']")];
      const preview = [...document.querySelectorAll("button")].find(
        (el) => el.textContent?.trim() === "Preview",
      );
      const label = preview?.querySelector("span");
      if (!tablist || tabs.length < 2 || !preview || !label) {
        return [{ name: "trajectory", delta: 999, details: "missing target" }];
      }
      const tablistStyle = getComputedStyle(tablist);
      const tablistRight = rect(tablist).right - px(tablistStyle.paddingRight);
      const lastTabRight = rect(tabs[tabs.length - 1]).right;
      const buttonRect = rect(preview);
      const labelRect = rect(label);
      return [
        {
          name: "trajectory tabs",
          delta: Math.abs(tablistRight - lastTabRight),
          details: `tablistRight=${tablistRight} lastTabRight=${lastTabRight}`,
        },
        {
          name: "trajectory preview button",
          delta: Math.abs(
            labelRect.left -
              buttonRect.left -
              (buttonRect.right - labelRect.right),
          ),
          details: `button=${buttonRect.left},${buttonRect.right} label=${labelRect.left},${labelRect.right}`,
        },
      ];
    });

    for (const edge of trajectoryEdges) {
      const maxDelta = edge.name.includes("button") ? 1 : 2;
      expect(edge.delta, edge.details).toBeLessThanOrEqual(maxDelta);
    }

    await compressionTab.click();

    const llmCompressEdges = await page.evaluate<EdgeMeasurement[]>(() => {
      const rect = (el: Element) => el.getBoundingClientRect();
      const rebuild = [...document.querySelectorAll("button")].find(
        (el) => el.textContent?.trim() === "Rebuild context",
      );
      const label = rebuild?.querySelector("span");
      if (!rebuild || !label) {
        return [
          { name: "trajectory rebuild", delta: 999, details: "missing target" },
        ];
      }
      const buttonRect = rect(rebuild);
      const labelRect = rect(label);
      return [
        {
          name: "trajectory rebuild button",
          delta: Math.abs(
            labelRect.left -
              buttonRect.left -
              (buttonRect.right - labelRect.right),
          ),
          details: `button=${buttonRect.left},${buttonRect.right} label=${labelRect.left},${labelRect.right}`,
        },
      ];
    });

    for (const edge of llmCompressEdges) {
      expect(edge.delta, edge.details).toBeLessThanOrEqual(1);
    }

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

    const modelGutters = await page.evaluate<GutterMeasurement[]>(() => {
      const dialog = document.querySelector("[role='dialog']");
      const row = document.querySelector(
        "[role='option'][data-selected='true']",
      );
      return [
        {
          name: "model selector popover content",
          value: dialog ? getComputedStyle(dialog).scrollbarGutter : "missing",
        },
        {
          name: "model selector selected row",
          value: row ? getComputedStyle(row).scrollbarGutter : "missing",
        },
      ];
    });

    for (const gutter of modelGutters) {
      expect(gutter.value, gutter.name).toBe("auto");
    }

    const modelSearch = modelPopover.getByRole("searchbox");
    await modelSearch.focus();
    await expect(modelSearch).toBeFocused();
    await modelSearch.press("Escape");
    await expect(models).toHaveCount(0);
    await expect(modelPopover).toHaveCount(0);
    await expect(modelTrigger).toHaveAttribute("aria-expanded", "false");
    await expect(modelTrigger).toBeFocused();
    await page.mouse.move(0, 0);
    await page.getByRole("button", { name: /Agent/ }).click();
    await page.getByRole("dialog").waitFor();

    const modeGutters = await page.evaluate<GutterMeasurement[]>(() => {
      const list = document.querySelector("[class*='modeList']");
      const selected = document.querySelector("[class*='itemSelected']");
      return [
        {
          name: "mode select list",
          value: list ? getComputedStyle(list).scrollbarGutter : "missing",
        },
        {
          name: "mode select selected item",
          value: selected
            ? getComputedStyle(selected).scrollbarGutter
            : "missing",
        },
      ];
    });

    for (const gutter of modeGutters) {
      expect(gutter.value, gutter.name).toBe("auto");
    }
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
