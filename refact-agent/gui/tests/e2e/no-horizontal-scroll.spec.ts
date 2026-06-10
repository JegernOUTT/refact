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
    name: "buddy",
    path: "/tests/e2e/route-showcase.html?route=buddy",
  },
  {
    name: "settings general",
    path: "/tests/e2e/route-showcase.html?route=settings&settings=general",
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
          await expect(page.getByTestId("buddy-home-hero")).toBeVisible();
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
          const buddyInnerOverflow = await page.evaluate<
            BuddyInnerOverflowReport[]
          >(() => {
            const targets = [
              {
                selector: "[data-testid='buddy-home-content']",
                description: "Buddy content scroller",
              },
              {
                selector: "[data-testid='buddy-home-hero']",
                description: "Buddy hero band",
              },
              {
                selector: "[data-testid='buddy-summary-strip']",
                description: "Buddy summary strip",
              },
              {
                selector: "[data-testid='buddy-opportunities-feed']",
                description: "Buddy opportunities feed",
              },
              {
                selector: "[data-testid='buddy-pulse-card']",
                description: "Buddy pulse card",
              },
              {
                selector: "[data-testid='buddy-personality-panel']",
                description: "Buddy personality panel",
              },
              {
                selector: "[data-testid='buddy-activity-panel']",
                description: "Buddy activity panel",
              },
              {
                selector: "[data-testid='buddy-recent-errors-panel']",
                description: "Buddy recent errors panel",
              },
              {
                selector: "[data-testid='buddy-workshop']",
                description: "Buddy workshop dock",
              },
            ];
            return targets.map(({ selector, description }) => {
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
              return {
                selector,
                description,
                missing: false,
                overflowing: el.scrollWidth > el.clientWidth + 2,
                scrollWidth: el.scrollWidth,
                clientWidth: el.clientWidth,
              };
            });
          });

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

    await page.getByLabel("Compress or Handoff").click();
    await page.getByRole("dialog").waitFor();

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

    await page.getByRole("tab", { name: "Handoff" }).click();

    const handoffEdges = await page.evaluate<EdgeMeasurement[]>(() => {
      const rect = (el: Element) => el.getBoundingClientRect();
      const create = [...document.querySelectorAll("button")].find(
        (el) => el.textContent?.trim() === "Create",
      );
      const label = create?.querySelector("span");
      if (!create || !label) {
        return [
          { name: "trajectory create", delta: 999, details: "missing target" },
        ];
      }
      const buttonRect = rect(create);
      const labelRect = rect(label);
      return [
        {
          name: "trajectory create button",
          delta: Math.abs(
            labelRect.left -
              buttonRect.left -
              (buttonRect.right - labelRect.right),
          ),
          details: `button=${buttonRect.left},${buttonRect.right} label=${labelRect.left},${labelRect.right}`,
        },
      ];
    });

    for (const edge of handoffEdges) {
      expect(edge.delta, edge.details).toBeLessThanOrEqual(1);
    }

    await page.keyboard.press("Escape");
    await page.getByLabel("Select model").click();
    await page.getByRole("listbox", { name: "Models" }).waitFor();

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

    await page.keyboard.press("Escape");
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
          value: selected ? getComputedStyle(selected).scrollbarGutter : "missing",
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

  test("renders main regions and toggles activity/settings without clipping", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 360, height: 900 });
    await page.goto("/tests/e2e/route-showcase.html?route=buddy");
    await page.getByTestId("buddy-home-content").waitFor();

    await expect(page.getByTestId("buddy-home-hero")).toBeVisible();
    await expect(page.getByTestId("buddy-world")).toBeVisible();
    await expect(page.getByTestId("buddy-summary-strip")).toBeVisible();
    await expect(page.getByTestId("buddy-opportunities-feed")).toBeVisible();
    await expect(page.getByTestId("buddy-pulse-card")).toBeVisible();
    await expect(page.getByTestId("buddy-activity-panel")).toBeVisible();

    await page.getByText("buddy_*").click();
    await expect(
      page.getByText("Buddy reviewed responsive surfaces"),
    ).toBeVisible();
    await expect(page.getByText("Refact e2e gate queued")).not.toBeVisible();

    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(page.getByTestId("buddy-settings-panel")).toBeVisible();

    const clipped = await page.evaluate<string[]>(() => {
      const viewportWidth = document.documentElement.clientWidth;
      return [
        "[data-testid='buddy-settings-panel']",
        "[data-testid='buddy-activity-panel']",
      ].flatMap((selector) => {
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
    });

    expect(
      clipped,
      `horizontally clipped Buddy panels: ${clipped.join(" | ")}`,
    ).toEqual([]);
  });
});
