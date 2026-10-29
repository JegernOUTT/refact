import { expect, test } from "@playwright/test";

const THEMES = ["light", "dark"] as const;
const BUDDY_ROUTE = "/tests/e2e/route-showcase.html?route=buddy";

type Rgba = { red: number; green: number; blue: number; alpha: number };

function parseCssColor(value: string): Rgba | null {
  const matches = value.match(/[\d.]+/g);
  if (!matches || matches.length < 3) return null;
  return {
    red: Number(matches[0]),
    green: Number(matches[1]),
    blue: Number(matches[2]),
    alpha: matches[3] === undefined ? 1 : Number(matches[3]),
  };
}

function linearChannel(value: number) {
  const normalized = value / 255;
  return normalized <= 0.04045
    ? normalized / 12.92
    : ((normalized + 0.055) / 1.055) ** 2.4;
}

function luminance(color: Rgba) {
  return (
    0.2126 * linearChannel(color.red) +
    0.7152 * linearChannel(color.green) +
    0.0722 * linearChannel(color.blue)
  );
}

function blend(foreground: Rgba, background: Rgba): Rgba {
  return {
    red:
      foreground.red * foreground.alpha +
      background.red * (1 - foreground.alpha),
    green:
      foreground.green * foreground.alpha +
      background.green * (1 - foreground.alpha),
    blue:
      foreground.blue * foreground.alpha +
      background.blue * (1 - foreground.alpha),
    alpha: 1,
  };
}

function contrastRatio(foreground: Rgba, background: Rgba) {
  const foregroundLuminance = luminance(foreground);
  const backgroundLuminance = luminance(background);
  return (
    (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
    (Math.min(foregroundLuminance, backgroundLuminance) + 0.05)
  );
}

for (const theme of THEMES) {
  test(`keeps the native decision error surface tinted and readable in ${theme}`, async ({
    page,
  }) => {
    await page.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      if (url.pathname === "/v1/buddy/approvals/current") {
        await route.fulfill({
          json: {
            schema_version: 1,
            project_root: "/workspace/refact",
            engine_identity: "showcase-engine",
            coverage: "loaded_native_sessions",
            truncated: false,
            warnings: [],
            sessions: [],
          },
        });
        return;
      }
      if (url.pathname === "/v1/buddy/decisions/current") {
        await route.fulfill({ status: 503, json: { detail: "unavailable" } });
        return;
      }
      await route.continue();
    });
    await page.goto(`${BUDDY_ROUTE}&theme=${theme}`);

    const errorRow = page
      .getByRole("region", { name: "Questions" })
      .getByRole("alert");
    await expect(errorRow).toBeVisible();
    const colors = await errorRow.evaluate((row) => {
      const text = row.querySelector<HTMLElement>("[class*='errorTitle']");
      if (!text) return null;
      const rowStyle = getComputedStyle(row);
      const textStyle = getComputedStyle(text);
      const backgrounds: string[] = [];
      for (
        let parent = row.parentElement;
        parent;
        parent = parent.parentElement
      )
        backgrounds.push(getComputedStyle(parent).backgroundColor);
      return {
        backgrounds,
        background: rowStyle.backgroundColor,
        foreground: textStyle.color,
        parentBackground: getComputedStyle(document.documentElement)
          .getPropertyValue("--rf-surface-1")
          .trim(),
      };
    });
    expect(colors).not.toBeNull();
    if (!colors) return;

    expect(colors.background).not.toBe("rgba(0, 0, 0, 0)");

    const foreground = parseCssColor(colors.foreground);
    const background = parseCssColor(colors.background);
    const parentBackground = parseCssColor(colors.parentBackground);
    expect(foreground).not.toBeNull();
    expect(background).not.toBeNull();
    if (background) expect(background.alpha).toBeGreaterThan(0);
    expect(parentBackground).not.toBeNull();
    if (!foreground || !background || !parentBackground) return;

    const backdrop = colors.backgrounds.reverse().reduce<Rgba>(
      (base, color) => {
        const parsed = parseCssColor(color);
        return parsed ? blend(parsed, base) : base;
      },
      { red: 255, green: 255, blue: 255, alpha: 1 },
    );
    const effectiveBackground = blend(background, backdrop);
    const effectiveForeground = blend(foreground, effectiveBackground);
    expect(
      contrastRatio(effectiveForeground, effectiveBackground),
    ).toBeGreaterThanOrEqual(4.5);
  });
}
