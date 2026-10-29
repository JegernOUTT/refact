import { expect, test, type Page } from "@playwright/test";

const BUDDY_ROUTE = "/tests/e2e/route-showcase.html?route=buddy&scene_speech=1";
const THEMES = ["dark", "light"] as const;
const WIDTHS = [360, 1280] as const;

type Rgba = { red: number; green: number; blue: number; alpha: number };

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

function contrastRatio(foreground: Rgba, background: Rgba) {
  const foregroundLuminance = luminance(foreground);
  const backgroundLuminance = luminance(background);
  return (
    (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
    (Math.min(foregroundLuminance, backgroundLuminance) + 0.05)
  );
}

async function openBuddy(
  page: Page,
  theme: (typeof THEMES)[number],
  width: (typeof WIDTHS)[number],
) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto(`${BUDDY_ROUTE}&theme=${theme}`);
  await page.getByTestId("buddy-home-content").waitFor();
  await page.getByRole("tab", { name: "Play", exact: true }).click();
  await expect(page.getByTestId("buddy-speech-bubble")).toBeVisible();
}

async function sceneTextContrast(page: Page) {
  return page.evaluate(() => {
    type ParsedColor = {
      red: number;
      green: number;
      blue: number;
      alpha: number;
    };
    type ContrastFailure = {
      text: string;
      ratio: number;
      required: number;
    };
    const parseColor = (value: string): ParsedColor | null => {
      const matches = value.match(/[\d.]+/g);
      if (!matches || matches.length < 3) return null;
      return {
        red: Number(matches[0]),
        green: Number(matches[1]),
        blue: Number(matches[2]),
        alpha: matches[3] === undefined ? 1 : Number(matches[3]),
      };
    };
    const blend = (foreground: ParsedColor, background: ParsedColor) => ({
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
    });
    const relativeLuminance = (color: ParsedColor) => {
      const linear = (channel: number) => {
        const normalized = channel / 255;
        return normalized <= 0.04045
          ? normalized / 12.92
          : ((normalized + 0.055) / 1.055) ** 2.4;
      };
      return (
        0.2126 * linear(color.red) +
        0.7152 * linear(color.green) +
        0.0722 * linear(color.blue)
      );
    };
    const effectiveBackground = (element: HTMLElement) => {
      const colors: ParsedColor[] = [];
      let current: HTMLElement | null = element;
      while (current) {
        const color = parseColor(getComputedStyle(current).backgroundColor);
        if (color && color.alpha > 0) colors.push(color);
        if (color && color.alpha >= 1) break;
        current = current.parentElement;
      }
      return colors
        .reverse()
        .reduce((background, color) => blend(color, background), {
          red: 255,
          green: 255,
          blue: 255,
          alpha: 1,
        });
    };
    const scene = document.querySelector("[data-testid='buddy-world']");
    if (!scene) return [];
    const walker = document.createTreeWalker(scene, NodeFilter.SHOW_TEXT);
    const failures: ContrastFailure[] = [];
    let node: Node | null;
    while ((node = walker.nextNode())) {
      const text = node.textContent?.trim();
      const element = node.parentElement;
      if (!text || !element) continue;
      const style = getComputedStyle(element);
      const bounds = element.getBoundingClientRect();
      if (
        style.visibility === "hidden" ||
        style.display === "none" ||
        bounds.width === 0 ||
        bounds.height === 0
      ) {
        continue;
      }
      const foreground = parseColor(style.color);
      if (!foreground) continue;
      const background = effectiveBackground(element);
      const compositedForeground = blend(foreground, background);
      const foregroundLuminance = relativeLuminance(compositedForeground);
      const backgroundLuminance = relativeLuminance(background);
      const ratio =
        (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
        (Math.min(foregroundLuminance, backgroundLuminance) + 0.05);
      const fontSize = Number.parseFloat(style.fontSize);
      const fontWeight = Number.parseInt(style.fontWeight, 10);
      const required =
        fontSize >= 24 || (fontSize >= 18.66 && fontWeight >= 700) ? 3 : 4.5;
      if (ratio < required) {
        failures.push({ text, ratio: Number(ratio.toFixed(2)), required });
      }
    }
    return failures;
  });
}

test.describe("Buddy scene contrast", () => {
  for (const theme of THEMES) {
    for (const width of WIDTHS) {
      test(`keeps every scene text node AA readable at ${width}px in ${theme}`, async ({
        page,
      }) => {
        await openBuddy(page, theme, width);

        expect(await sceneTextContrast(page)).toEqual([]);
      });
    }
  }

  test("speech_bubble_is_readable_in_dark", async ({ page }) => {
    await openBuddy(page, "dark", 1280);

    const colors = await page
      .getByTestId("buddy-speech-bubble")
      .evaluate((element) => {
        const skin = element.firstElementChild;
        if (!(skin instanceof HTMLElement)) return null;
        const style = getComputedStyle(skin);
        return { foreground: style.color, background: style.backgroundColor };
      });
    expect(colors).not.toBeNull();
    if (colors === null) return;
    const foreground = parseCssColor(colors.foreground);
    const background = parseCssColor(colors.background);
    expect(foreground).not.toBeNull();
    expect(background).not.toBeNull();
    if (foreground === null || background === null) return;
    const ratio = contrastRatio(foreground, background);

    expect(ratio).toBeGreaterThanOrEqual(4.5);
  });

  test("scene_labels_pass_in_light", async ({ page }) => {
    await openBuddy(page, "light", 1280);

    const hotspot = page
      .getByTestId("buddy-world")
      .locator("button[aria-label]")
      .first();
    await hotspot.focus();
    const tooltip = page.getByRole("tooltip");
    await expect(tooltip).toBeVisible();
    const colors = await tooltip.evaluate((element) => {
      const skin =
        element.closest("[data-state][class]") ?? element.parentElement!;
      const style = getComputedStyle(skin);
      return { foreground: style.color, background: style.backgroundColor };
    });
    const foreground = parseCssColor(colors.foreground);
    const background = parseCssColor(colors.background);
    expect(foreground).not.toBeNull();
    expect(background).not.toBeNull();
    if (foreground && background)
      expect(contrastRatio(foreground, background)).toBeGreaterThanOrEqual(4.5);
  });
});

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
