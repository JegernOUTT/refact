import {
  chromium,
  expect,
  test,
  type BrowserContext,
  type Locator,
  type Page,
  type TestInfo,
} from "@playwright/test";
import { existsSync } from "node:fs";
import {
  createProjectRoot,
  Daemon,
  fixtureEnvironment,
  seedChat,
  seedLegacyProfile,
  startSite,
  type Project,
} from "./project-browser/fixture";

test.skip(
  !process.env.REFACT_PROJECT_BROWSER_SCRATCH,
  "Real daemon and Chromium: use tests/e2e/project-browser/run.sh",
);

const region = (page: Page) =>
  page.getByRole("region", { name: "Project browser" });
const address = (page: Page) =>
  region(page).getByRole("textbox", { name: /address|url/i });
const frame = (page: Page) =>
  region(page).getByRole("img", { name: /browser/i });
const browserTab = (page: Page) =>
  page.getByRole("tab", { name: "Browser", exact: true });
const chatTab = (page: Page, title: string) =>
  page.getByRole("tab").filter({
    has: page.getByText(title, { exact: true }),
  });
const openButton = (page: Page) =>
  page.getByRole("button", { name: "Open browser", exact: true });
const browserEndpoint = "/v1/browser/project/";

type BrowserFrame = {
  runtime_id: string;
  tab_id: string;
  url: string;
  title: string;
  width: number;
  height: number;
  viewport: { width: number; height: number };
};

function observeFrames(page: Page) {
  let latest: BrowserFrame | undefined;
  page.on("response", (response) => {
    if (
      response.ok() &&
      new URL(response.url()).pathname.endsWith(`${browserEndpoint}frame`)
    ) {
      void response
        .json()
        .then((body: BrowserFrame) => {
          latest = body;
        })
        .catch(() => undefined);
    }
  });
  return () => latest;
}

async function openGui(
  daemon: Daemon,
  context: BrowserContext,
  project: Project,
  chatId: string,
) {
  const url = await daemon.installProjectCookie(context, project.project_id);
  const page = await context.newPage();
  await page.goto(`${url}?chat=${chatId}`);
  return page;
}

async function openBrowser(page: Page) {
  const composer = page.getByTestId("chat-form-textarea");
  await composer.click();
  await openButton(page).click();
  await expect(browserTab(page)).toBeVisible();
  await expect(region(page)).toBeVisible();
}

async function navigate(
  page: Page,
  url: string,
  observe: ReturnType<typeof observeFrames>,
) {
  await address(page).fill(url);
  await address(page).press("Enter");
  await expect.poll(() => observe()?.url, { timeout: 20_000 }).toBe(url);
  await expect(frame(page)).toBeVisible();
}

async function picture(page: Page, info: TestInfo, name: string) {
  const path = info.outputPath(`${name}.png`);
  await page.screenshot({ path, animations: "disabled" });
  await info.attach(name, { path, contentType: "image/png" });
}

async function framePoint(
  image: Locator,
  state: BrowserFrame,
  x: number,
  y: number,
) {
  const rect = await image.boundingBox();
  if (!rect) throw new Error("Browser frame has no bounds");
  const scale = Math.min(rect.width / state.width, rect.height / state.height);
  const width = state.width * scale;
  const height = state.height * scale;
  return {
    x: rect.x + (rect.width - width) / 2 + (x / state.viewport.width) * width,
    y:
      rect.y +
      (rect.height - height) / 2 +
      (y / state.viewport.height) * height,
  };
}

async function frameShowsFooter(image: Locator) {
  return image.evaluate(async (element: HTMLImageElement) => {
    await element.decode();
    const canvas = document.createElement("canvas");
    canvas.width = element.naturalWidth;
    canvas.height = element.naturalHeight;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) throw new Error("No GUI frame decode canvas");
    context.drawImage(element, 0, 0);
    const [red, green, blue] = context.getImageData(
      canvas.width - 25,
      canvas.height - 25,
      1,
      1,
    ).data;
    return red < 40 && green > 105 && blue > 80;
  });
}

async function switchChat(page: Page, title: string) {
  const tab = chatTab(page, title);
  if (!(await tab.isVisible())) {
    await page.getByRole("button", { name: "Home", exact: true }).click();
    await page.getByText(title, { exact: true }).click();
  } else {
    await tab.click();
  }
  await expect(tab).toHaveAttribute("aria-selected", "true");
}

test("daemon-served Browser inputs, migration, shared chat runtime and persistent project isolation", async ({
  playwright,
}, info) => {
  const fixture = fixtureEnvironment();
  const daemon = new Daemon(fixture);
  const site = await startSite();
  const alphaRoot = await createProjectRoot(fixture, "project-alpha");
  const betaRoot = await createProjectRoot(fixture, "project-beta");
  let gui: Awaited<ReturnType<typeof chromium.launch>> | undefined;
  let page: Page | undefined;
  let alpha: Project;
  let alphaChat = "";
  let beforeRestart: BrowserFrame | undefined;

  try {
    await test.step("start fresh daemon, Chromium, and synthetic same-project legacy profiles", async () => {
      await daemon.start(site.origin);
      alpha = await daemon.openProject(alphaRoot);
      alphaChat = await seedChat(daemon, alpha, "Alpha chat one");
      await seedChat(daemon, alpha, "Alpha chat two");
      const selected = await seedLegacyProfile(
        fixture,
        alpha,
        site.origin,
        "selected",
      );
      const unselected = await seedLegacyProfile(
        fixture,
        alpha,
        site.origin,
        "unselected",
      );
      gui = await playwright.chromium.launch({
        executablePath: fixture.chrome,
        env: fixture.env,
      });
      const context = await gui.newContext({
        viewport: { width: 1500, height: 900 },
        serviceWorkers: "block",
      });
      page = await openGui(daemon, context, alpha, alphaChat);
      await expect(chatTab(page, "Alpha chat one")).toHaveAttribute(
        "aria-selected",
        "true",
        { timeout: 25_000 },
      );
      await openBrowser(page);
      const panel = region(page);
      const picker = panel.getByRole("combobox", {
        name: "Chat browser profile",
      });
      await expect(picker).toBeVisible({ timeout: 20_000 });
      await expect(
        picker.getByRole("option", { name: unselected.id, exact: true }),
      ).toBeEnabled();
      expect(existsSync(selected.path)).toBe(true);
      expect(existsSync(unselected.path)).toBe(true);
      const adopt = panel.getByRole("button", { name: "Adopt profile" });
      await expect(adopt).toBeDisabled();
      await picker.selectOption(selected.id);
      await adopt.click();
      await expect.poll(() => existsSync(selected.path)).toBe(false);
      expect(existsSync(unselected.path)).toBe(true);
    });

    if (!page || !gui) throw new Error("GUI fixture did not start");
    const originalPage = page;
    const observe = observeFrames(originalPage);

    await test.step("navigate a static site, type and click a native form, wheel the rendered frame", async () => {
      await navigate(originalPage, `${site.origin}/form`, observe);
      const formVisit = site.visits
        .filter((visit) => visit.path === "/form")
        .at(-1);
      expect(formVisit?.userAgent).toMatch(/Chrome\//);
      expect(formVisit?.cookie).toContain("fixture_legacy=selected");
      expect(formVisit?.cookie).not.toContain("unselected");
      const state = observe();
      if (!state) throw new Error("No Chromium frame after navigation");
      const image = frame(originalPage);
      const input = await framePoint(image, state, 60, 86);
      const submit = await framePoint(image, state, 215, 86);
      await originalPage.mouse.click(input.x, input.y);
      await originalPage.keyboard.type("typed in actual Browser");
      await originalPage.mouse.click(submit.x, submit.y);
      await expect
        .poll(
          () =>
            site.visits.filter((visit) => visit.path === "/submit").at(-1)
              ?.value,
        )
        .toBe("typed in actual Browser");
      await expect
        .poll(() => observe()?.title)
        .toBe("Browser fixture · submitted");
      await expect.poll(() => frameShowsFooter(image)).toBe(false);
      await originalPage.mouse.move(input.x, input.y + 140);
      await originalPage.mouse.wheel(0, 1150);
      await expect
        .poll(() => frameShowsFooter(image), { timeout: 15_000 })
        .toBe(true);
      await picture(originalPage, info, "01-static-site-scrolled");
    });

    await test.step("resize, close and reopen the Browser tab, then switch between real chats", async () => {
      const initial = observe();
      if (!initial) throw new Error("No browser frame before resizing");
      const initialWidth = initial.viewport.width;
      const size = await region(originalPage).boundingBox();
      if (!size) throw new Error("Browser pane has no bounds before resizing");
      await originalPage.setViewportSize({ width: 1080, height: 760 });
      await expect
        .poll(async () => {
          const next = await region(originalPage).boundingBox();
          return Math.abs((next?.width ?? size.width) - size.width);
        })
        .toBeGreaterThan(20);
      await expect
        .poll(() =>
          Math.abs((observe()?.viewport.width ?? initialWidth) - initialWidth),
        )
        .toBeGreaterThan(20);
      await expect(frame(originalPage)).toBeVisible();
      const id = observe()?.runtime_id;
      const tabId = observe()?.tab_id;
      await originalPage
        .getByRole("button", { name: "Close Browser", exact: true })
        .click();
      await expect(browserTab(originalPage)).toHaveCount(0);
      await openBrowser(originalPage);
      await expect(frame(originalPage)).toBeVisible({ timeout: 15_000 });
      await expect(address(originalPage)).toHaveValue(/\/submit\?entry=/);
      expect(observe()?.runtime_id).toBe(id);
      expect(observe()?.tab_id).toBe(tabId);
      await switchChat(originalPage, "Alpha chat two");
      await switchChat(originalPage, "Alpha chat one");
      await expect(browserTab(originalPage)).toBeVisible();
      if (!(await address(originalPage).isVisible()))
        await browserTab(originalPage).click();
      await expect(address(originalPage)).toHaveValue(/\/submit\?entry=/);
      expect(observe()?.runtime_id).toBe(id);
      expect(observe()?.tab_id).toBe(tabId);
      beforeRestart = observe();
      await picture(originalPage, info, "02-browser-after-chat-switch");
    });

    await test.step("restart daemon and worker; reload using a fresh GUI context and verify cookie survives", async () => {
      await originalPage.context().close();
      await daemon.stop();
      await daemon.start(site.origin);
      const restarted = await daemon.openProject(alphaRoot);
      expect(restarted.project_id).toBe(alpha.project_id);
      expect(restarted.worker.pid).not.toBe(alpha.worker.pid);
      const context = await gui!.newContext({
        viewport: { width: 1500, height: 900 },
        serviceWorkers: "block",
      });
      const restartedPage = await openGui(
        daemon,
        context,
        restarted,
        alphaChat,
      );
      page = restartedPage;
      const observeRestarted = observeFrames(restartedPage);
      await openBrowser(restartedPage);
      const before = site.visits.length;
      await navigate(restartedPage, `${site.origin}/check`, observeRestarted);
      const visit = site.visits
        .slice(before)
        .find(({ path }) => path === "/check");
      expect(visit?.cookie).toContain("fixture_login=project-alpha");
      expect(visit?.cookie).toContain("fixture_legacy=selected");
      expect(observeRestarted()?.runtime_id).not.toBe(
        beforeRestart?.runtime_id,
      );
      expect(observeRestarted()?.title).toBe("Signed in: Project Alpha");
      await picture(restartedPage, info, "03-project-alpha-after-restart");
    });

    await test.step("same fixture host in a different project sees no cookie", async () => {
      const beta = await daemon.openProject(betaRoot);
      expect(beta.project_id).not.toBe(alpha.project_id);
      const betaChat = await seedChat(daemon, beta, "Beta chat");
      const context = await gui!.newContext({
        viewport: { width: 1500, height: 900 },
        serviceWorkers: "block",
      });
      const betaPage = await openGui(daemon, context, beta, betaChat);
      page = betaPage;
      const observeBeta = observeFrames(betaPage);
      await openBrowser(betaPage);
      const before = site.visits.length;
      await navigate(betaPage, `${site.origin}/check`, observeBeta);
      const visit = site.visits
        .slice(before)
        .find(({ path }) => path === "/check");
      expect(visit?.cookie).toBe("");
      expect(observeBeta()?.title).toBe("Signed out");
      await picture(betaPage, info, "04-project-beta-isolated");
      expect(
        site.visits.filter(({ path }) => path === "/model-must-not-be-invoked"),
      ).toEqual([]);
    });
  } catch (error) {
    await info.attach("daemon-output", {
      body: daemon.diagnostics(),
      contentType: "text/plain",
    });
    if (page && !page.isClosed()) await picture(page, info, "failure-gui");
    throw error;
  } finally {
    try {
      await gui?.close();
    } finally {
      try {
        await daemon.stop();
      } finally {
        await site.close();
      }
    }
  }
});
