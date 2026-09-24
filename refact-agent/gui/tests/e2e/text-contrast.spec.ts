import { expect, test, type Page } from "@playwright/test";

const PROJECT = { project_id: "showcase", canonical_root: "/workspace/refact" };
const BUDDY_ROUTES = [
  { name: "buddy-enabled", mode: "enabled" },
  { name: "buddy-disabled", mode: "disabled" },
  { name: "buddy-unavailable", mode: "unavailable" },
] as const;
const OTHER_ROUTES = [
  { name: "chat", path: "chat" },
  { name: "settings", path: "settings" },
  { name: "workspace-chrome", path: "workspace-chrome" },
] as const;
const WIDTHS = [360, 1280] as const;
const THEMES = ["dark", "light"] as const;
const ARTIFACT_ID = "contrast-report";
const WORK_ID = "contrast-work";
const COLOR_NAMES = ["gray", "red", "orange", "accent"] as const;
type ColorName = (typeof COLOR_NAMES)[number];
type Measurement = {
  text: string;
  className: string;
  color: ColorName;
  ratio: number;
  required: number;
};
const themeSummary: Record<
  (typeof THEMES)[number],
  { nodes: Record<ColorName, number>; minimumRatio: number }
> = {
  dark: {
    nodes: { gray: 0, red: 0, orange: 0, accent: 0 },
    minimumRatio: Infinity,
  },
  light: {
    nodes: { gray: 0, red: 0, orange: 0, accent: 0 },
    minimumRatio: Infinity,
  },
};
const routeSummary: Record<
  string,
  Record<(typeof THEMES)[number], Record<ColorName, number>>
> = {};

function envelope(data: unknown) {
  return { schema_version: 1, project: PROJECT, data, next_cursor: null };
}

function record(value: unknown) {
  return { value, revision: 1, change_seq: 1 };
}

async function installBuddyRoutes(page: Page) {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    if (!path.startsWith("/v1/")) return route.continue();
    if (path === "/v1/buddy/work") {
      const work = {
        schema_version: 1,
        id: WORK_ID,
        project: PROJECT,
        subject: {
          resource_kind: "saved_report",
          resource_id: "contrast report",
          version: "1",
        },
        kind: "assessment",
        title: "Readability contrast report",
        state: "needs_attention",
        native_sources: [],
        current_artifact_id: ARTIFACT_ID,
        current_artifact_revision: 1,
        last_material_change_at: new Date().toISOString(),
      };
      return route.fulfill({ json: envelope([record(work)]) });
    }
    if (path === `/v1/buddy/artifacts/${ARTIFACT_ID}`) {
      const artifact = {
        schema_version: 1,
        artifact_id: ARTIFACT_ID,
        revision: 1,
        project: PROJECT,
        source_run_ids: [],
        title: "Readability contrast report",
        conclusion: "The saved report is available for review.",
        delta: "No changes are applied automatically.",
        body: "The report fixture contains a bounded saved summary.",
        uncertainty: "This saved result does not cover every route.",
        proposed_action_ids: [],
        evidence: [],
        provenance: {
          source_run_ids: [],
          evidence_refs: [],
          contains_untrusted_source: false,
        },
      };
      return route.fulfill({ json: envelope(record(artifact)) });
    }
    if (path === `/v1/buddy/work/${WORK_ID}/actions`) {
      return route.fulfill({
        status: 503,
        json: { detail: "Fixture action error" },
      });
    }
    if (
      path === `/v1/buddy/work/${WORK_ID}/read` ||
      path === `/v1/buddy/work/${WORK_ID}/snooze`
    ) {
      return route.fulfill({ json: envelope(null) });
    }
    if (
      path === "/v1/buddy/decisions/current" ||
      path === "/v1/buddy/approvals/current" ||
      path === "/v1/buddy/supervision/context" ||
      path === "/v1/buddy/native-work"
    ) {
      return route.fulfill({ status: 503, json: { detail: "Fixture only" } });
    }
    return route.continue();
  });
}

async function coloredTextContrast(page: Page) {
  return page.evaluate(() => {
    type Color = { red: number; green: number; blue: number; alpha: number };
    const parseColor = (value: string): Color | null => {
      const values = value.match(/[\d.]+/g);
      if (!values || values.length < 3) return null;
      return {
        red: Number(values[0]),
        green: Number(values[1]),
        blue: Number(values[2]),
        alpha: values[3] === undefined ? 1 : Number(values[3]),
      };
    };
    const blend = (foreground: Color, background: Color): Color => ({
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
    const luminance = (color: Color) => {
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
      const layers: Color[] = [];
      let current: HTMLElement | null = element;
      while (current) {
        const background = parseColor(
          getComputedStyle(current).backgroundColor,
        );
        if (background && background.alpha > 0) layers.push(background);
        if (background && background.alpha >= 1) break;
        current = current.parentElement;
      }
      return layers
        .reverse()
        .reduce((background, layer) => blend(layer, background), {
          red: 255,
          green: 255,
          blue: 255,
          alpha: 1,
        });
    };
    const measurements: Measurement[] = [];
    const elements = [...document.querySelectorAll<HTMLElement>("*")];
    for (const element of elements) {
      const color = (["gray", "red", "orange", "accent"] as const).find(
        (name) =>
          [...element.classList].some((className) =>
            new RegExp(`(?:^|_)color-${name}(?:_|$)`).test(className),
          ),
      );
      if (!color) continue;
      const style = getComputedStyle(element);
      const bounds = element.getBoundingClientRect();
      if (
        !element.textContent?.trim() ||
        style.visibility === "hidden" ||
        style.display === "none" ||
        bounds.width === 0 ||
        bounds.height === 0
      ) {
        continue;
      }
      let ancestor: HTMLElement | null = element;
      let hiddenByOpacity = false;
      while (ancestor) {
        if (Number(getComputedStyle(ancestor).opacity) === 0) {
          hiddenByOpacity = true;
          break;
        }
        ancestor = ancestor.parentElement;
      }
      if (hiddenByOpacity) continue;
      const foreground = parseColor(style.color);
      if (!foreground) continue;
      const background = effectiveBackground(element);
      const compositedForeground = blend(foreground, background);
      const foregroundLuminance = luminance(compositedForeground);
      const backgroundLuminance = luminance(background);
      const ratio =
        (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
        (Math.min(foregroundLuminance, backgroundLuminance) + 0.05);
      const fontSize = Number.parseFloat(style.fontSize);
      const fontWeight = Number.parseInt(style.fontWeight, 10);
      const required =
        fontSize >= 24 || (fontSize >= 18.66 && fontWeight >= 700) ? 3 : 4.5;
      measurements.push({
        text: element.innerText.trim().replace(/\s+/g, " "),
        className: [...element.classList]
          .filter((className) => className.includes(`color-${color}`))
          .join(" "),
        color,
        ratio: Number(ratio.toFixed(2)),
        required,
      });
    }
    return measurements;
  });
}

async function checkContrast(
  page: Page,
  routeName: string,
  width: number,
  theme: (typeof THEMES)[number],
  mustMeasure: boolean,
) {
  routeSummary[routeName] ??= {
    dark: { gray: 0, red: 0, orange: 0, accent: 0 },
    light: { gray: 0, red: 0, orange: 0, accent: 0 },
  };
  const measurements = await coloredTextContrast(page);
  if (mustMeasure) {
    expect(
      measurements.length,
      `${routeName} ${width}px ${theme} must render colored Text nodes`,
    ).toBeGreaterThan(0);
  }
  const failures = measurements.filter(
    (measurement) => measurement.ratio < measurement.required,
  );
  for (const measurement of measurements) {
    routeSummary[routeName][theme][measurement.color] += 1;
    themeSummary[theme].nodes[measurement.color] += 1;
    themeSummary[theme].minimumRatio = Math.min(
      themeSummary[theme].minimumRatio,
      measurement.ratio,
    );
  }
  const counts = Object.fromEntries(
    COLOR_NAMES.map((color) => [
      color,
      measurements.filter((measurement) => measurement.color === color).length,
    ]),
  );
  console.info(
    `${routeName} ${width}px ${theme}: ${
      measurements.length
    } colored Text nodes (${JSON.stringify(counts)}); minimum ${
      measurements.length
        ? Math.min(...measurements.map((entry) => entry.ratio))
        : "n/a"
    }:1`,
  );
  expect(
    failures,
    `${routeName} ${width}px ${theme} contrast failures: ${JSON.stringify(
      failures,
    )}`,
  ).toEqual([]);
}

for (const { name, mode } of BUDDY_ROUTES) {
  for (const width of WIDTHS) {
    for (const theme of THEMES) {
      test(`${name} colored Text passes AA at ${width}px in ${theme}`, async ({
        page,
      }) => {
        await installBuddyRoutes(page);
        await page.setViewportSize({ width, height: 900 });
        await page.goto(
          `/tests/e2e/route-showcase.html?route=buddy&operator=${mode}&theme=${theme}`,
        );
        await page.getByTestId("buddy-home-content").waitFor();
        if (mode === "enabled") {
          const row = page.getByRole("button", {
            name: /Readability contrast report/,
          });
          await expect(row).toBeVisible();
          await row.click();
          await expect(page.getByLabel("Work detail")).toBeVisible();
          await expect(page.getByLabel("Registered actions")).toBeVisible();
          await expect(page.getByText("What this did not cover")).toBeVisible();
          await checkContrast(
            page,
            `${name} report detail`,
            width,
            theme,
            true,
          );
          await page
            .getByRole("tab", { name: "Settings", exact: true })
            .click();
          await expect(
            page.getByText("SETTINGS", { exact: true }),
          ).toBeVisible();
        } else {
          await page
            .getByRole("tab", { name: "Settings", exact: true })
            .click();
          await expect(
            page.getByText("SETTINGS", { exact: true }),
          ).toBeVisible();
        }
        await checkContrast(page, name, width, theme, true);
      });
    }
  }
}

for (const route of OTHER_ROUTES) {
  for (const width of WIDTHS) {
    for (const theme of THEMES) {
      test(`${route.name} colored Text contrast at ${width}px in ${theme}`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.goto(
          `/tests/e2e/route-showcase.html?route=${route.path}&theme=${theme}`,
        );
        await page.locator("[data-element='app-root']").waitFor();
        if (route.name === "chat") {
          const settingsTrigger = page
            .locator("[class*='bottomModelControl'] button")
            .first();
          if (await settingsTrigger.isVisible()) {
            await settingsTrigger.click();
          }
          const threadInfo = page.getByRole("button", { name: "Thread info" });
          if (await threadInfo.isVisible()) await threadInfo.click();
          const trajectory = page.getByRole("button", {
            name: "Compress or rebuild context",
          });
          if (await trajectory.count()) await trajectory.click();
        } else if (route.name === "settings") {
          await page.getByRole("heading", { name: "Settings" }).waitFor();
        }
        await checkContrast(page, route.name, width, theme, false);
      });
    }
  }
}

test.afterAll(() => {
  console.info(`Text contrast route counts: ${JSON.stringify(routeSummary)}`);
  for (const theme of THEMES) {
    const summary = themeSummary[theme];
    console.info(
      `Text contrast sweep ${theme} totals: ${JSON.stringify(
        summary.nodes,
      )}; minimum ratio ${
        Number.isFinite(summary.minimumRatio) ? summary.minimumRatio : "n/a"
      }:1`,
    );
  }
});
