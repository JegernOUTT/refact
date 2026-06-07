// Harness choice: this gate runs against a minimal Vite-served route showcase
// instead of Storybook or the live app entrypoint. It renders real dashboard and
// chat React surfaces with mocked Redux state, so CI does not need a running LSP.
import { expect, test } from "@playwright/test";

type OverflowReport = {
  docOverflow: boolean;
  offenders: string[];
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

        const overflow = await page.evaluate<OverflowReport>(() => {
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
            .map((el) => {
              if (typeof el.className === "string" && el.className) {
                return el.className;
              }
              return el.tagName.toLowerCase();
            });
          return { docOverflow, offenders };
        });

        expect(
          overflow.docOverflow,
          `offenders: ${overflow.offenders.join(" | ")}`,
        ).toBe(false);
      });
    }
  }
});
