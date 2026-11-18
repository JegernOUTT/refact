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
import { spawn } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import {
  createProjectRoot,
  createDesktopOpener,
  Daemon,
  fixtureEnvironment,
  seedChat,
  seedLegacyProfile,
  startSite,
  type DesktopOpener,
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
const browserTabs = (page: Page) =>
  region(page).getByRole("group", { name: "Browser tabs" });
const chatTab = (page: Page, title: string) =>
  page.getByRole("tab").filter({
    has: page.getByText(title, { exact: true }),
  });
const openButton = (page: Page) =>
  page.getByRole("button", { name: "Open browser", exact: true });
const browserEndpoint = "/v1/browser/project/";
const alphaSignedIn = "Signed in: Project Alpha";

type BrowserFrame = {
  runtime_id: string;
  tab_id: string;
  url: string;
  title: string;
  width: number;
  height: number;
  viewport: { width: number; height: number };
};

type BrowserStatus = {
  project_id: string | null;
  connected: boolean;
  running: boolean;
  runtime_id: string | null;
  active_tab: string | null;
  tabs: { tab_id: string; title: string; url: string }[];
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

test("Linux refact ui handoff opens the real project Browser without leaking launch authority", async ({
  playwright,
}, info) => {
  test.skip(process.platform !== "linux", "Linux desktop opener acceptance");
  const fixture = fixtureEnvironment();
  const daemon = new Daemon(fixture);
  const site = await startSite();
  const opener = await createDesktopOpener(fixture);
  const alphaRoot = await createProjectRoot(fixture, "handoff-alpha");
  const betaRoot = await createProjectRoot(fixture, "handoff-beta");
  const browserProcesses: number[] = [];
  const checkUrl = `${site.origin}/check`;
  const launch = (project: Project, index: number) =>
    launchDesktop(daemon, opener, project, index, browserProcesses);
  const observations: Record<
    string,
    ReturnType<typeof observeDesktopBrowser>
  > = {};
  try {
    await daemon.start(site.origin, { authEnabled: false, opener });
    const project = await daemon.openProject(alphaRoot);
    await seedChat(daemon, project, "Handoff alpha chat");

    const ordinaryShell = await fetch(
      `${daemon.origin}/p/${project.project_id}/`,
      { redirect: "manual" },
    );
    expect(ordinaryShell.status).toBe(200);
    await expectBareBrowserDenied(daemon, project);

    const firstLaunch = await launch(project, 0);
    const first = await connectDesktopChromium(
      playwright.chromium,
      firstLaunch.profile,
      firstLaunch.url,
    );
    const initial = (observations.alpha = observeDesktopBrowser(
      first.page,
      firstLaunch.url,
    ));
    let submitted: BrowserFrame;
    try {
      await openSeededChat(first.page, "Handoff alpha chat");
      await openBrowserFromDesktop(first.page);
      await initial.ready(project);
      await navigate(first.page, `${site.origin}/form`, initial.frame);
      const state = initial.frame();
      if (!state) throw new Error("No Browser frame after local handoff");
      const image = frame(first.page);
      const field = await framePoint(image, state, 60, 86);
      const submit = await framePoint(image, state, 215, 86);
      await first.page.mouse.click(field.x, field.y);
      await first.page.keyboard.type("real local handoff");
      await first.page.mouse.click(submit.x, submit.y);
      await expect
        .poll(
          () => isLocalHandoffSubmission(initial.frame()?.url, site.origin),
          {
            timeout: 20_000,
          },
        )
        .toBe(true);
      const submission = initial.frame();
      if (!submission) throw new Error("No submitted Browser frame");
      submitted = submission;
      expect(submitted.title).toBe("Browser fixture · submitted");
      expect(
        site.visits.some(
          ({ path, value }) =>
            path === "/submit" && value === "real local handoff",
        ),
      ).toBe(true);

      await browserTabs(first.page)
        .locator('button[aria-label="New tab"]')
        .click();
      await expect
        .poll(
          () => {
            const active = initial.status()?.active_tab;
            return Boolean(active && active !== submitted.tab_id);
          },
          { timeout: 20_000 },
        )
        .toBe(true);
      await checkSiteLogin(first.page, site, checkUrl, initial.frame, true);
      const closeCheck = browserTabs(first.page).getByRole("button", {
        name: `Close tab: ${alphaSignedIn}`,
        exact: true,
      });
      await closeCheck.click();
      await expect(closeCheck).toHaveCount(0);
      await selectBrowserTab(first.page, submitted.title);
      await expect
        .poll(() => initial.frame()?.tab_id, { timeout: 20_000 })
        .toBe(submitted.tab_id);
      await expect(address(first.page)).toHaveValue(submitted.url);
      await expectNoDaemonToken(first, daemon);
      await picture(first.page, info, "05-linux-local-launch");
    } finally {
      await first.context.close();
      await first.browser.close();
    }

    await daemon.stop();
    await daemon.start(site.origin, { authEnabled: false, opener });
    const restarted = await daemon.openProject(alphaRoot);
    expect(restarted.project_id).toBe(project.project_id);
    const secondLaunch = await launch(restarted, 1);
    expect(secondLaunch.file === firstLaunch.file).toBe(false);
    expect(secondLaunch.url === firstLaunch.url).toBe(false);
    const second = await connectDesktopChromium(
      playwright.chromium,
      secondLaunch.profile,
      secondLaunch.url,
    );
    const restored = (observations.alphaRelaunch = observeDesktopBrowser(
      second.page,
      secondLaunch.url,
    ));
    try {
      await openSeededChat(second.page, "Handoff alpha chat");
      await openBrowserFromDesktop(second.page);
      await restored.ready(restarted);
      let restoredTab: BrowserStatus["tabs"][number] | undefined;
      await expect
        .poll(
          () => {
            restoredTab = restored
              .status()
              ?.tabs.find(({ url }) => url === submitted.url);
            return Boolean(restoredTab);
          },
          {
            message:
              "Chromium must list the previously submitted URL before any GUI navigation",
            timeout: 25_000,
          },
        )
        .toBe(true);
      if (!restoredTab) throw new Error("No restored Browser tab");
      const restoredTabId = restoredTab.tab_id;
      if (restored.status()?.active_tab !== restoredTabId) {
        await selectBrowserTab(second.page, restoredTab.title || submitted.url);
      }
      await expect
        .poll(
          () => {
            const current = restored.frame();
            return (
              current?.tab_id === restoredTabId && current.url === submitted.url
            );
          },
          { timeout: 25_000 },
        )
        .toBe(true);
      expect(restored.frame()?.runtime_id !== submitted.runtime_id).toBe(true);
      expect(
        restored.navigationCount(),
        "Restoration must not issue a Browser navigate request",
      ).toBe(0);
      await expect(address(second.page)).toHaveValue(submitted.url);
      await expect(frame(second.page)).toBeVisible();
      await picture(second.page, info, "06-linux-restored-tab");

      await checkSiteLogin(second.page, site, checkUrl, restored.frame, true);
      await expectNoDaemonToken(second, daemon);
      await picture(second.page, info, "07-linux-alpha-after-restart");
    } finally {
      await second.context.close();
      await second.browser.close();
    }

    const beta = await daemon.openProject(betaRoot);
    expect(beta.project_id).not.toBe(restarted.project_id);
    await expectBareBrowserDenied(daemon, beta);
    await seedChat(daemon, beta, "Handoff beta chat");
    const betaLaunch = await launch(beta, 2);
    expect(
      new Set([firstLaunch.file, secondLaunch.file, betaLaunch.file]).size,
    ).toBe(3);
    expect(
      new Set([firstLaunch.url, secondLaunch.url, betaLaunch.url]).size,
    ).toBe(3);
    const other = await connectDesktopChromium(
      playwright.chromium,
      betaLaunch.profile,
      betaLaunch.url,
    );
    const isolated = (observations.beta = observeDesktopBrowser(
      other.page,
      betaLaunch.url,
    ));
    try {
      await openSeededChat(other.page, "Handoff beta chat");
      await openBrowserFromDesktop(other.page);
      await isolated.ready(beta);
      await checkSiteLogin(other.page, site, checkUrl, isolated.frame, false);
      await expectNoDaemonToken(other, daemon);
      await picture(other.page, info, "08-linux-beta-isolated");
    } finally {
      await other.context.close();
      await other.browser.close();
    }
    expect(
      site.visits.filter(({ path }) => path === "/model-must-not-be-invoked"),
    ).toEqual([]);
    expect(
      (await readFile(opener.arguments, "utf8")).split(/\r?\n/).filter(Boolean)
        .length,
    ).toBe(3);
    expect(daemon.loggedToken()).toBe(false);
  } catch (error) {
    for (const [label, observe] of Object.entries(observations)) {
      await info.attach(`${label}-browser-state`, {
        body: observe.diagnostics(site.origin),
        contentType: "application/json",
      });
    }
    throw error;
  } finally {
    for (const pid of browserProcesses) {
      try {
        process.kill(pid, "SIGTERM");
      } catch {
        // Browser may already have exited after closing its CDP connection.
      }
    }
    try {
      await daemon.stop();
    } finally {
      await site.close();
    }
  }
});

function observeDesktopBrowser(page: Page, launchUrl: string) {
  const observeFrame = observeFrames(page);
  const projectUrl = new URL(launchUrl);
  const endpoint = `${projectUrl.pathname}v1/browser/project/`;
  let status: BrowserStatus | undefined;
  let statusHttp: number | undefined;
  let frameHttp: number | undefined;
  let sawHandoffStatus = false;
  let navigations = 0;
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (
      url.origin === projectUrl.origin &&
      url.pathname === `${endpoint}navigate`
    ) {
      navigations += 1;
    }
  });
  page.on("response", (response) => {
    const url = new URL(response.url());
    if (url.origin !== projectUrl.origin) return;
    if (url.pathname === `${endpoint}frame`) frameHttp = response.status();
    if (url.pathname !== `${endpoint}status`) return;
    statusHttp = response.status();
    const request = response.request();
    if (
      !response.ok() ||
      request.method() !== "POST" ||
      !request.headers()["x-refact-browser-launch"]
    )
      return;
    sawHandoffStatus = true;
    void response
      .json()
      .then((body: BrowserStatus) => {
        status = body;
      })
      .catch(() => undefined);
  });
  return {
    frame: observeFrame,
    status: () => status,
    navigationCount: () => navigations,
    async ready(project: Project) {
      await expect
        .poll(
          () =>
            sawHandoffStatus &&
            status?.project_id === project.project_id.slice(0, 16),
          {
            message:
              "The desktop handoff must authorize a successful Browser status request for this project",
            timeout: 20_000,
          },
        )
        .toBe(true);
      await expect
        .poll(
          () => {
            const current = observeFrame();
            return Boolean(
              status?.running &&
                status.connected &&
                status.runtime_id &&
                status.active_tab &&
                current?.runtime_id === status.runtime_id &&
                current.tab_id === status.active_tab,
            );
          },
          {
            message:
              "The Browser must return a frame for the active tab of its running runtime",
            timeout: 25_000,
          },
        )
        .toBe(true);
      await expect(address(page)).toBeVisible();
      await expect(frame(page)).toBeVisible();
    },
    diagnostics(siteOrigin: string) {
      const safeId = (id: string | null | undefined) =>
        !id ? null : /^[a-f0-9-]{8,36}$/i.test(id) ? id : "[redacted id]";
      const safeTab = ({ tab_id, url }: { tab_id: string; url: string }) => {
        const parsed = URL.canParse(url) ? new URL(url) : undefined;
        return {
          tabId: safeId(tab_id),
          url:
            url === "about:blank"
              ? url
              : parsed?.origin === siteOrigin &&
                  ["/form", "/check", "/submit"].includes(parsed.pathname)
                ? parsed.pathname
                : "[redacted URL]",
        };
      };
      const latestFrame = observeFrame();
      return JSON.stringify(
        {
          statusHttp,
          frameHttp,
          sawHandoffStatus,
          navigations,
          running: status?.running,
          connected: status?.connected,
          runtimeId: safeId(status?.runtime_id),
          activeTabId: safeId(status?.active_tab),
          tabs: status?.tabs.map(safeTab) ?? [],
          frame: latestFrame
            ? {
                runtimeId: safeId(latestFrame.runtime_id),
                ...safeTab(latestFrame),
              }
            : null,
        },
        null,
        2,
      );
    },
  };
}

async function checkSiteLogin(
  page: Page,
  site: Awaited<ReturnType<typeof startSite>>,
  checkUrl: string,
  observe: ReturnType<typeof observeFrames>,
  signedIn: boolean,
) {
  const before = site.visits.length;
  await navigate(page, checkUrl, observe);
  const visit = site.visits.slice(before).find(({ path }) => path === "/check");
  expect(Boolean(visit), "Browser must send a fresh /check request").toBe(true);
  expect(Boolean(visit?.userAgent.includes("Chrome/"))).toBe(true);
  expect(
    signedIn
      ? visit?.cookie.split("; ").includes("fixture_login=project-alpha")
      : visit?.cookie === "",
    "Browser /check must have the expected fixture login state",
  ).toBe(true);
  await expect
    .poll(
      () =>
        observe()?.url === checkUrl &&
        observe()?.title === (signedIn ? alphaSignedIn : "Signed out"),
      { timeout: 20_000 },
    )
    .toBe(true);
}

async function selectBrowserTab(page: Page, label: string) {
  const tab = browserTabs(page).getByRole("button", {
    name: label,
    exact: true,
  });
  if ((await tab.getAttribute("aria-pressed")) !== "true") await tab.click();
  await expect(tab).toHaveAttribute("aria-pressed", "true");
}

async function expectBareBrowserDenied(daemon: Daemon, project: Project) {
  const response = await fetch(
    `${daemon.origin}/p/${project.project_id}${browserEndpoint}status`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{}",
      redirect: "manual",
    },
  );
  expect([401, 403, 404]).toContain(response.status);
  expect(response.headers.has("location")).toBe(false);
}

async function expectNoDaemonToken(
  desktop: Awaited<ReturnType<typeof connectDesktopChromium>>,
  daemon: Daemon,
) {
  const cookies = await desktop.context.cookies();
  expect(cookies.some(({ value }) => value === daemon.token)).toBe(false);
  const storage = await desktop.page.evaluate(() => ({
    localStorage: Object.values(localStorage),
    sessionStorage: Object.values(sessionStorage),
  }));
  expect(JSON.stringify(storage).includes(daemon.token)).toBe(false);
}

function isLocalHandoffSubmission(url: string | undefined, origin: string) {
  if (!url || !URL.canParse(url)) return false;
  const submitted = new URL(url);
  return (
    submitted.origin === origin &&
    submitted.pathname === "/submit" &&
    submitted.searchParams.get("entry") === "real local handoff"
  );
}

async function openSeededChat(page: Page, title: string) {
  await page.getByRole("button", { name: "Home", exact: true }).click();
  const link = page.getByRole("link", { name: title, exact: true });
  await expect(link).toBeVisible({ timeout: 20_000 });
  await link.click();
  await expect(page.getByTestId("chat-form-textarea")).toBeVisible({
    timeout: 20_000,
  });
}

async function openBrowserFromDesktop(page: Page) {
  if (await browserTab(page).count()) {
    await expect(region(page)).toBeVisible();
    await expect(address(page)).toBeVisible();
    return;
  }
  const composer = page.getByTestId("chat-form-textarea");
  await composer.fill("local fixture setup");
  await expect(openButton(page)).toBeVisible();
  await openButton(page).click();
  await expect(browserTab(page)).toBeVisible();
  await expect(region(page)).toBeVisible();
}

async function launchDesktop(
  daemon: Daemon,
  opener: DesktopOpener,
  project: Project,
  index: number,
  pids: number[],
) {
  const url = (await spawnUi(daemon, project.root)).trim();
  const pid = Number(await readOneLine(opener.pids, index));
  if (!Number.isSafeInteger(pid) || pid <= 1)
    throw new Error("Desktop opener did not report a valid Chromium PID");
  pids.push(pid);
  expect(await readOneLine(opener.argumentCounts, index)).toBe("1");
  const file = await readOneLine(opener.arguments, index);
  expect(/^\/[^?#]*\.html$/.test(file) && !file.includes(daemon.token)).toBe(
    true,
  );
  expect((await readFile(file, "utf8")).includes(daemon.token)).toBe(false);
  expect(
    /^http:\/\/refact-[a-f0-9]+\.localhost:\d+\/p\/[^?#/]+\/$/.test(url) &&
      url.endsWith(`/p/${project.project_id}/`) &&
      !url.includes(daemon.token),
    "refact ui must print only the clean launch URL for the selected project",
  ).toBe(true);
  const profile = await readOneLine(opener.profilePaths, index);
  await writeFile(join(profile, "fixture-ready"), "", { mode: 0o600 });
  return { url, file, profile };
}

function spawnUi(daemon: Daemon, root: string): Promise<string> {
  const child = spawn(daemon.fixtureEngine, ["ui", root], {
    cwd: root,
    env: daemon.uiEnv,
    stdio: ["ignore", "pipe", "ignore"],
  });
  return new Promise((resolve, reject) => {
    let stdout = "";
    child.stdout?.on("data", (chunk: Buffer) => (stdout += chunk.toString()));
    child.once("error", reject);
    child.once("close", (code) => {
      if (code === 0) resolve(stdout);
      else reject(new Error(`refact ui exited with ${code}`));
    });
  });
}

async function readOneLine(path: string, index = 0): Promise<string> {
  const end = Date.now() + 20_000;
  while (Date.now() < end) {
    try {
      const lines = (await readFile(path, "utf8"))
        .split(/\r?\n/)
        .filter(Boolean);
      if (lines[index]) return lines[index];
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    await delay(100);
  }
  throw new Error("Timed out waiting for desktop opener observation");
}

async function connectDesktopChromium(
  browserType: typeof chromium,
  profile: string,
  expectedUrl: string,
) {
  const endpoint = await readDevtoolsEndpoint(profile);
  const browser = await browserType.connectOverCDP(endpoint);
  const context = browser.contexts()[0];
  if (!context) throw new Error("Opened Chromium has no default context");
  const page = context.pages()[0] ?? (await context.newPage());
  await page.setViewportSize({ width: 1500, height: 900 });
  await expect
    .poll(() => page.url() === expectedUrl, {
      message: "The desktop handoff must reach the clean project shell URL",
      timeout: 30_000,
    })
    .toBe(true);
  return { browser, context, page };
}

async function readDevtoolsEndpoint(profile: string): Promise<string> {
  const path = join(profile, "DevToolsActivePort");
  const end = Date.now() + 30_000;
  while (Date.now() < end) {
    try {
      const lines = (await readFile(path, "utf8")).split(/\r?\n/);
      if (lines[0] && lines[1]) return `http://127.0.0.1:${lines[0]}`;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    await delay(100);
  }
  throw new Error("Timed out waiting for the real Chromium DevTools endpoint");
}
