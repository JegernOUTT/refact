// Harness choice: this gate runs against a minimal Vite-served route showcase
// instead of Storybook or the live app entrypoint. It renders real dashboard and
// chat React surfaces with mocked Redux state, so CI does not need a running LSP.
import { expect, test } from "@playwright/test";

type OverflowReport = {
  docOverflow: boolean;
  offenders: string[];
  innerScrollOffenders: string[];
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
          route.path.includes("route=settings"),
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
      });
    }
  }
});
