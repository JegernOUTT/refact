import {
  expect,
  test,
  type Browser,
  type BrowserContext,
  type Locator,
  type Page,
  type TestInfo,
} from "@playwright/test";
import { createHash } from "node:crypto";
import { existsSync, readdirSync } from "node:fs";
import { cp, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";
import {
  createProjectRoot,
  Daemon,
  fixtureEnvironment,
  NON_LOOPBACK_HOST,
  seedLegacyProfile,
  startSite,
  Worker,
  writeChat,
  writeEngineConfig,
  type Fixture,
  type Site,
} from "./project-browser/fixture";

test.skip(
  !process.env.REFACT_PROJECT_BROWSER_SCRATCH,
  "Real engine and Chromium: use tests/e2e/project-browser/run.sh",
);

const browserEndpoint = "/v1/browser/project/";
const baseline = process.env.REFACT_PROJECT_BROWSER_BASELINE === "1";

type BrowserFrame = {
  runtime_id: string;
  tab_id: string;
  url: string;
  title: string;
  width: number;
  height: number;
  viewport: { width: number; height: number };
};

type StatusTab = {
  tab_id: string;
  url: string;
  title: string;
  name: string | null;
  opened_by_chat: string | null;
  last_chat: string | null;
};

type BrowserStatus = {
  running: boolean;
  connected: boolean;
  runtime_id?: string;
  tabs: StatusTab[];
};

const composer = (page: Page) => page.getByTestId("chat-form-textarea");
const browserView = (page: Page) =>
  page.getByRole("region", { name: "Browser tab", exact: true });
const frameImage = (page: Page) =>
  browserView(page).getByRole("img", { name: "Browser page" });
const workspaceTab = (page: Page, title: string) =>
  page.getByRole("tab", {
    name: new RegExp(`(^|\\s)${title.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`),
  });
const leafSections = (page: Page) =>
  page.locator("section[data-workspace-leaf-id]");
const railButton = (page: Page) =>
  page.locator('button[aria-controls="workspace-browser-panel"]');
const dockPanel = (page: Page) => page.locator("#workspace-browser-panel");

/** Every response the GUI page receives, plus the latest Browser frame per tab. */
/** Local wall-clock time in the worker log format (HHMMSS.mmm). */
const logClock = (at: number) => {
  const time = new Date(at);
  const pad = (value: number, size = 2) => String(value).padStart(size, "0");
  return `${pad(time.getHours())}${pad(time.getMinutes())}${pad(
    time.getSeconds(),
  )}.${pad(time.getMilliseconds(), 3)}`;
};

type BrowserRequest = {
  route: string;
  tab_id: string | null;
  runtime_id: string | null;
  kind: string | null;
  started: string;
  ended: string | null;
  startAt: number;
  endAt: number | null;
  ms: number | null;
  status: number | null;
  responseAt: number | null;
  failure: string | null;
};

async function observe(page: Page) {
  const frames = new Map<string, BrowserFrame>();
  const frameLog: { tab_id: string; url: string; title: string; at: number }[] =
    [];
  const streamErrors: {
    tab_id: string;
    status: number | null;
    detail: string;
  }[] = [];
  await page.exposeBinding(
    "__projectBrowserRecord",
    (
      _source,
      record: {
        type: string;
        frame?: BrowserFrame;
        error?: { tab_id: string; status: number | null; detail: string };
      },
    ) => {
      if (record.error) streamErrors.push(record.error);
      if (record.type === "frame" && record.frame) {
        const frame = record.frame;
        frames.set(frame.tab_id, frame);
        if (frameLog.length === 128) frameLog.shift();
        frameLog.push({
          tab_id: frame.tab_id,
          url: frame.url,
          title: frame.title,
          at: Date.now(),
        });
      }
    },
  );
  await page.addInitScript(() => {
    const nativeFetch = window.fetch;
    window.fetch = async (...args) => {
      const response = await nativeFetch(...args);
      const signal =
        args[1]?.signal ??
        (args[0] instanceof Request ? args[0].signal : undefined);
      if (
        !response.url.includes("/v1/browser/project/frames/subscribe") ||
        !response.ok ||
        !response.body
      )
        return response;
      const reader = response.clone().body!.getReader();
      const body =
        typeof args[1]?.body === "string"
          ? (JSON.parse(args[1].body) as { tab_id?: string })
          : {};
      const tabId = body.tab_id ?? "";
      signal?.addEventListener(
        "abort",
        () => {
          void reader.cancel();
        },
        { once: true },
      );
      const report = (record: unknown) =>
        (
          window as unknown as {
            __projectBrowserRecord: (record: unknown) => Promise<void>;
          }
        ).__projectBrowserRecord(record);
      void (async () => {
        let buffer = new Uint8Array(0);
        try {
          for (;;) {
            const { value, done } = await reader.read();
            if (done) break;
            const next = new Uint8Array(buffer.length + value.length);
            next.set(buffer);
            next.set(value, buffer.length);
            buffer = next;
            while (buffer.length >= 8) {
              const view = new DataView(
                buffer.buffer,
                buffer.byteOffset,
                buffer.byteLength,
              );
              const header = view.getUint32(0);
              const payload = view.getUint32(4);
              if (header > 65536 || payload > 16 * 1024 * 1024)
                throw new Error("oversized stream record");
              const length = 8 + header + payload;
              if (buffer.length < length) break;
              const parsed = JSON.parse(
                new TextDecoder().decode(buffer.subarray(8, 8 + header)),
              ) as { type: string };
              if (parsed.type === "frame") {
                if (!payload) throw new Error("empty stream frame");
                await report({ type: "frame", frame: parsed });
              } else if (parsed.type === "error") {
                await report({
                  type: "error",
                  error: {
                    tab_id: tabId,
                    status: (parsed as { status?: number }).status ?? null,
                    detail: JSON.stringify(parsed),
                  },
                });
              } else if (parsed.type !== "heartbeat" || payload !== 0)
                throw new Error("invalid stream record");
              buffer = buffer.slice(length);
            }
          }
          if (buffer.length && !signal?.aborted)
            throw new Error("truncated stream record");
        } catch (error) {
          if (!signal?.aborted)
            await report({
              type: "observer-error",
              error: { tab_id: tabId, status: null, detail: String(error) },
            });
        } finally {
          reader.releaseLock();
        }
      })();
      return response;
    };
  });
  const requests: BrowserRequest[] = [];
  const pending = new Map<object, { entry: BrowserRequest; at: number }>();
  const byRequest = new Map<object, BrowserRequest>();
  const finish = (
    request: object,
    status: number | null,
    failure: string | null,
  ) => {
    const open = pending.get(request);
    if (!open) return;
    pending.delete(request);
    const at = Date.now();
    open.entry.ended = logClock(at);
    open.entry.endAt = at;
    open.entry.ms = at - open.at;
    if (status !== null) open.entry.status = status;
    open.entry.failure = failure;
  };
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (!url.pathname.includes(browserEndpoint)) return;
    const body = JSON.parse(request.postData() || "{}") as {
      tab_id?: string;
      runtime_id?: string;
      kind?: string;
      op?: string;
    };
    const at = Date.now();
    const entry: BrowserRequest = {
      route: url.pathname.slice(
        url.pathname.indexOf(browserEndpoint) + browserEndpoint.length,
      ),
      tab_id: body.tab_id ?? null,
      runtime_id: body.runtime_id ?? null,
      kind: body.kind ?? body.op ?? null,
      started: logClock(at),
      ended: null,
      startAt: at,
      endAt: null,
      ms: null,
      status: null,
      responseAt: null,
      failure: null,
    };
    requests.push(entry);
    byRequest.set(request, entry);
    pending.set(request, { entry, at });
  });
  page.on("requestfinished", (request) => {
    void request
      .response()
      .then((response) => finish(request, response?.status() ?? null, null));
  });
  page.on("requestfailed", (request) =>
    finish(request, null, request.failure()?.errorText ?? "failed"),
  );
  const responses: { path: string; status: number }[] = [];
  const failures: {
    path: string;
    status: number;
    request: string | null;
    body: string;
    at: number;
  }[] = [];
  page.on("response", (response) => {
    const url = new URL(response.url());
    responses.push({ path: url.pathname, status: response.status() });
    const entry = byRequest.get(response.request());
    if (entry) {
      entry.status = response.status();
      entry.responseAt = Date.now();
    }
    if (
      baseline &&
      response.ok() &&
      url.pathname.endsWith(`${browserEndpoint}frame`)
    ) {
      void response.json().then((frame: BrowserFrame) => {
        frames.set(frame.tab_id, frame);
        if (frameLog.length === 128) frameLog.shift();
        frameLog.push({
          tab_id: frame.tab_id,
          url: frame.url,
          title: frame.title,
          at: Date.now(),
        });
      });
    }
    if (url.pathname.includes(browserEndpoint) && response.status() >= 400) {
      void response
        .text()
        .catch(() => "")
        .then((body) =>
          failures.push({
            path: url.pathname,
            status: response.status(),
            request: response.request().postData(),
            body,
            at: Date.now(),
          }),
        );
    }
  });
  return {
    frame: (tabId: string) => frames.get(tabId),
    frameLog,
    streamErrors,
    responses,
    failures,
    requests,
    browserResponses: () =>
      responses.filter(({ path }) => path.includes(browserEndpoint)),
    /** 401/403 anywhere except the cloud-account organization endpoints, which need a
     * Refact cloud login unrelated to the Browser and answer 401 in every fixture. */
    denied: () =>
      responses.filter(
        ({ path, status }) =>
          (status === 401 || status === 403) &&
          !path.includes("/v1/organization/"),
      ),
  };
}
type Observer = Awaited<ReturnType<typeof observe>>;

function invalidSubscribeAttempts(
  requests: BrowserRequest[],
  closedAt: Map<string, number> = new Map(),
  restart?: { at: number; ready: number; identities: Set<string> },
) {
  return requests.filter((request) => {
    if (request.route !== "frames/subscribe") return false;
    const refusedDuringRestart =
      restart &&
      request.failure === "net::ERR_CONNECTION_REFUSED" &&
      request.startAt >= restart.at &&
      request.endAt !== null &&
      request.endAt <= restart.ready &&
      restart.identities.has(`${request.runtime_id}:${request.tab_id}`);
    if (request.status === 200)
      return request.failure !== null && request.failure !== "net::ERR_ABORTED";
    if (request.status === null) {
      if (request.failure === "net::ERR_ABORTED") return false;
      return !refusedDuringRestart;
    }
    const closed = closedAt.get(request.tab_id ?? "");
    const afterClose =
      closed !== undefined && (request.responseAt ?? 0) >= closed;
    const afterRestart =
      restart !== undefined &&
      restart.identities.has(`${request.runtime_id}:${request.tab_id}`) &&
      (request.responseAt ?? 0) >= restart.at;
    return !(
      (request.status === 410 && afterClose) ||
      ([404, 410].includes(request.status) && afterRestart)
    );
  });
}

test("subscription audit checks each attempt and lifecycle boundaries", () => {
  const attempt = (
    status: number | null,
    failure: string | null = null,
  ): BrowserRequest => ({
    route: "frames/subscribe",
    tab_id: "tab",
    runtime_id: "runtime",
    kind: null,
    started: "",
    ended: "",
    startAt: 10,
    endAt: 12,
    ms: 2,
    status,
    responseAt: status === null ? null : 11,
    failure,
  });
  const ok = attempt(200);
  for (const status of [201, 204, 409, 500]) {
    const bad = attempt(status, "net::ERR_ABORTED");
    expect(invalidSubscribeAttempts([bad, ok])).toEqual([bad]);
  }
  expect(
    invalidSubscribeAttempts([attempt(null, "net::ERR_ABORTED"), ok]),
  ).toEqual([]);
  expect(invalidSubscribeAttempts([attempt(null), ok])).toHaveLength(1);
  const brokenStream = attempt(200, "net::ERR_CONNECTION_RESET");
  expect(invalidSubscribeAttempts([brokenStream, ok])).toEqual([brokenStream]);
  const refused = attempt(null, "net::ERR_CONNECTION_REFUSED");
  const restart = { at: 10, ready: 12, identities: new Set(["runtime:tab"]) };
  expect(invalidSubscribeAttempts([refused], new Map(), restart)).toEqual([]);
  expect(
    invalidSubscribeAttempts([refused], new Map(), { ...restart, ready: 11 }),
  ).toEqual([refused]);
  expect(
    invalidSubscribeAttempts([refused], new Map(), {
      ...restart,
      identities: new Set(["other:tab"]),
    }),
  ).toEqual([refused]);
  const gone = attempt(410);
  expect(invalidSubscribeAttempts([gone], new Map([["tab", 12]]))).toEqual([
    gone,
  ]);
  expect(invalidSubscribeAttempts([gone], new Map([["tab", 11]]))).toEqual([]);
  expect(
    invalidSubscribeAttempts([gone], new Map(), { ...restart, at: 12 }),
  ).toEqual([gone]);
  expect(invalidSubscribeAttempts([gone], new Map(), restart)).toEqual([]);
});

const timeline: { name: string; at: number; local: string }[] = [];

async function picture(page: Page, info: TestInfo, name: string) {
  const at = Date.now();
  timeline.push({ name, at, local: logClock(at) });
  const path = info.outputPath(`${name}.png`);
  await page.screenshot({ path, animations: "disabled" });
  await info.attach(name, { path, contentType: "image/png" });
}

async function newGuiPage(browser: Browser) {
  const context = await browser.newContext({
    viewport: { width: 1600, height: 950 },
    serviceWorkers: "block",
  });
  const page = await context.newPage();
  return { context, page, net: await observe(page) };
}

async function sendMessage(page: Page, text: string) {
  await expect(composer(page)).toBeEditable({ timeout: 30_000 });
  await composer(page).fill(text);
  await composer(page).press("Enter");
  await expect(page.getByText(text, { exact: true }).first()).toBeVisible({
    timeout: 20_000,
  });
}

async function expectNoCredentialUi(page: Page) {
  await expect(
    page.getByText(
      /credential|unauthori[sz]ed|project session|sign in to (the )?browser|HTTP 401|401 unauthorized/i,
    ),
  ).toHaveCount(0);
}

/** The visible Browser view's Chromium tab id once it shows a frame matching `ready`. */
async function visibleBrowserTab(
  page: Page,
  net: Observer,
  ready: (frame: BrowserFrame) => boolean,
  timeout = 90_000,
) {
  await expect(browserView(page)).toBeVisible({ timeout });
  const tabId = await browserView(page).getAttribute("data-browser-tab-id");
  if (!tabId) throw new Error("Browser view has no tab id");
  await expect
    .poll(
      () => {
        const frame = net.frame(tabId);
        return frame ? ready(frame) : false;
      },
      { timeout, message: `Browser frame of ${tabId} never became ready` },
    )
    .toBe(true);
  await expect(frameImage(page)).toBeVisible();
  return tabId;
}

/** Chat pane on the left, Browser pane on the right, both visible side by side. */
async function expectBrowserRightOfChat(page: Page) {
  const chat = leafSections(page).filter({ has: composer(page) });
  const browser = leafSections(page).filter({ has: browserView(page) });
  await expect(chat).toHaveCount(1);
  await expect(browser).toHaveCount(1);
  await expect(composer(page)).toBeVisible();
  await expect(frameImage(page)).toBeVisible();
  const [left, right] = await Promise.all([
    chat.boundingBox(),
    browser.boundingBox(),
  ]);
  if (!left || !right) throw new Error("Split panes have no bounds");
  expect(left.width).toBeGreaterThan(250);
  expect(right.width).toBeGreaterThan(250);
  expect(left.x + left.width).toBeLessThanOrEqual(right.x + 2);
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

async function openChat(page: Page, title: string) {
  const tab = workspaceTab(page, title);
  if (await tab.isVisible()) {
    await tab.click();
  } else {
    await page.getByRole("button", { name: "Home", exact: true }).click();
    await page.getByRole("link", { name: title, exact: true }).click();
  }
  await expect(workspaceTab(page, title)).toHaveAttribute(
    "aria-selected",
    "true",
    { timeout: 20_000 },
  );
  await expect(composer(page)).toBeVisible();
}

async function inPagePost(page: Page, path: string) {
  return page.evaluate(async (url) => {
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{}",
    });
    return { status: response.status, body: await response.text() };
  }, path);
}

type RouteFailure = Observer["failures"][number];

/**
 * The engine answers 409 when a tab's gate is held by another user, the renderer misses its
 * deadline, or the page is changing; the GUI keeps the last frame and retries. Only these
 * exact details on /frame and /resize are transient by contract; any 409 on /input or
 * /navigate is not.
 */
const GATE_HOLDERS = [
  "a Browser-view frame",
  "a Browser-view resize",
  "a Browser-view input",
  "a Browser-view navigation",
  "another browser action",
] as const;

const TRANSIENT_RENDERER_DETAILS: Record<string, readonly string[]> = {
  frame: [
    "a frame of this tab is already being captured; try again",
    ...GATE_HOLDERS.filter((holder) => holder !== "a Browser-view frame").map(
      (holder) =>
        `this tab is busy with ${holder}; no frame was captured; try again`,
    ),
    "the browser did not return a frame in time; try again",
    "the page is changing; no frame was captured; try again",
  ],
  resize: [
    ...GATE_HOLDERS.map(
      (holder) =>
        `this tab is busy with ${holder}; the pane size was not applied; try again`,
    ),
    "the browser did not confirm the pane size in time; try again",
    "the page is changing; the pane size was not confirmed; try again",
  ],
};

function isTransientRendererConflict(failure: RouteFailure) {
  if (failure.status !== 409) return false;
  const route = failure.path.slice(
    failure.path.indexOf(browserEndpoint) + browserEndpoint.length,
  );
  if (route !== "frame" && route !== "resize") return false;
  let detail = "";
  try {
    detail = (JSON.parse(failure.body) as { detail?: string }).detail ?? "";
  } catch {
    return false;
  }
  return TRANSIENT_RENDERER_DETAILS[route]?.includes(detail) ?? false;
}

function recordTransientConflicts(info: TestInfo, observers: Observer[]) {
  for (const failure of observers.flatMap((net) => net.failures))
    if (isTransientRendererConflict(failure))
      info.annotations.push({
        type: "transient-409",
        description: `${logClock(failure.at)} ${failure.path} ${
          failure.request ?? ""
        } ${failure.body}`,
      });
}

async function writeRouteEvidence(info: TestInfo, observers: Observer[]) {
  await writeFile(
    info.outputPath("browser-route-failures.json"),
    JSON.stringify(
      {
        timeline,
        recentFrames: observers.map((net) => net.frameLog),
        streamErrors: observers.flatMap((net) => net.streamErrors),
        failures: observers.flatMap((net) =>
          net.failures.map((failure) => ({
            ...failure,
            local: logClock(failure.at),
            transient: isTransientRendererConflict(failure),
          })),
        ),
      },
      null,
      2,
    ),
  );
  await writeFile(
    info.outputPath("browser-requests.json"),
    JSON.stringify(
      observers.map((net) => net.requests),
      null,
      1,
    ),
  );
}

/** All error responses have been read (their bodies arrive asynchronously). */
async function settledFailures(net: Observer) {
  await expect
    .poll(
      () =>
        net.browserResponses().filter(({ status }) => status >= 400).length -
        net.failures.length,
    )
    .toBe(0);
}

/** Keeps process output, model calls and engine logs next to the screenshots. */
async function keepDiagnostics(
  info: TestInfo,
  fixture: Fixture,
  site: Site,
  outputs: Record<string, string>,
  roots: string[] = [],
) {
  for (const [name, body] of Object.entries(outputs))
    await writeFile(info.outputPath(`${name}-output.txt`), body);
  await writeFile(
    info.outputPath("llm-calls.json"),
    JSON.stringify(site.llmCalls, null, 2),
  );
  await writeFile(
    info.outputPath("site-visits.json"),
    JSON.stringify(site.visits, null, 2),
  );
  const logs = join(fixture.cache, "logs");
  if (existsSync(logs))
    await cp(logs, info.outputPath("engine-logs"), { recursive: true });
  for (const root of roots) {
    const trajectories = join(root, ".refact", "trajectories");
    if (existsSync(trajectories))
      await cp(
        trajectories,
        info.outputPath(`trajectories-${basename(root)}`),
        {
          recursive: true,
        },
      );
  }
}

/** The chat's tool messages exactly as the engine streams them to the GUI (snapshot event). */
async function wireToolMessages(apiBase: string, chatId: string) {
  const abort = new AbortController();
  const response = await fetch(
    `${apiBase}/v1/chats/subscribe?chat_id=${encodeURIComponent(chatId)}`,
    { signal: abort.signal },
  );
  if (!response.ok || !response.body)
    throw new Error(`subscribe: HTTP ${response.status}`);
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) throw new Error("subscribe stream ended without a snapshot");
      buffer += decoder.decode(value, { stream: true });
      for (const line of buffer.split("\n")) {
        if (!line.startsWith("data: ")) continue;
        let event: { type?: string; messages?: Record<string, unknown>[] };
        try {
          event = JSON.parse(line.slice(6)) as typeof event;
        } catch {
          continue;
        }
        if (event.type !== "snapshot") continue;
        return (event.messages ?? [])
          .filter((message) => message.role === "tool")
          .map((message) => ({
            tool_call_id: message.tool_call_id,
            keys: Object.keys(message).sort(),
            top_level_browser_project: message.browser_project ?? null,
            extra_browser_project:
              (message.extra as Record<string, unknown> | undefined)
                ?.browser_project ?? null,
          }));
      }
    }
  } finally {
    abort.abort();
  }
}

/**
 * Sends a scripted message, waits until the model has seen the chrome tool result, then
 * requires the Browser view to have opened on its own. If it did not, the failure is
 * recorded and the tab is opened from the Browser rail so later checks still run.
 */
async function browseAndExpectAutoOpen(
  page: Page,
  site: Site,
  message: string,
  view: { title?: string; railText: string | RegExp },
) {
  const script = /fixture script ([a-z0-9-]+)/.exec(message)?.[1];
  const finished = () =>
    site.llmCalls.filter(
      (call) => call.script === script && call.responded === "final",
    ).length;
  const before = finished();
  await sendMessage(page, message);
  await expect
    .poll(finished, {
      timeout: 90_000,
      message: "the model never received the chrome tool result",
    })
    .toBeGreaterThan(before);
  const shown = view.title ? workspaceTab(page, view.title) : browserView(page);
  await expect
    .soft(shown, "the Browser view must auto-open after the chrome tool call")
    .toBeVisible({ timeout: 20_000 });
  if (!(await shown.isVisible())) await openFromRail(page, view.railText);
}

async function openFromRail(page: Page, text: string | RegExp) {
  await railButton(page).click();
  const item = dockPanel(page)
    .getByRole("list", { name: "Browser tabs" })
    .locator("li")
    .filter({ hasText: text })
    .first();
  await item.getByRole("button").first().click();
  await railButton(page).click();
  await expect(browserView(page)).toBeVisible({ timeout: 20_000 });
}

async function closeAll(...items: (Browser | BrowserContext | undefined)[]) {
  for (const item of items) await item?.close().catch(() => undefined);
}

const waitTimeout = 60_000;

function scripts(site: Site) {
  return {
    login: site.script("login", {
      tab_name: "Fixture login",
      steps: [{ action: "navigate", url: `${site.origin}/login` }],
    }),
    check: site.script("check", {
      steps: [{ action: "navigate", url: `${site.origin}/check` }],
    }),
    second: site.script("second", {
      tab_name: "Second chat page",
      steps: [{ action: "navigate", url: `${site.origin}/second` }],
    }),
    plain: "Just answer, no browsing: fixture script plain",
    waitLogin: site.script("waitlogin", {
      steps: [
        { action: "navigate", url: `${site.origin}/login?wait=1` },
        {
          action: "wait_for_function",
          expression: `async () => {
            if (location.pathname === '/login' && !window.fixtureWaitObserved) {
              window.fixtureWaitObserved = true;
              await fetch('/wait-started');
            }
            return location.href === ${JSON.stringify(`${site.origin}/check`)};
          }`,
          timeout_ms: waitTimeout,
        },
      ],
    }),
  };
}

test("standalone worker on 0.0.0.0 via a non-loopback host: chat-driven Browser tabs, adoption, persistence and project isolation", async ({
  playwright,
}, info) => {
  test.setTimeout(900_000);
  const fixture = fixtureEnvironment();
  const site = await startSite();
  await writeEngineConfig(fixture, site.origin);
  const say = scripts(site);
  const alphaRoot = await createProjectRoot(fixture, "worker-alpha");
  const betaRoot = await createProjectRoot(fixture, "worker-beta");
  const chatOne = await writeChat(alphaRoot, "Alpha chat one");
  const chatTwo = await writeChat(alphaRoot, "Alpha chat two");
  const betaChat = await writeChat(betaRoot, "Beta chat");
  const hour = 3_600_000;
  const older = await seedLegacyProfile(
    fixture,
    alphaRoot,
    site,
    "older",
    new Date(Date.now() - 2 * hour),
  );
  const newer = await seedLegacyProfile(
    fixture,
    alphaRoot,
    site,
    "newer",
    new Date(Date.now() - hour),
  );
  const alpha = new Worker(fixture, alphaRoot);
  const beta = new Worker(fixture, betaRoot);
  let gui: Browser | undefined;
  let page: Page | undefined;
  const observers: Observer[] = [];
  let tabA = "";
  const closedTabs = new Set<string>();
  const closedAt = new Map<string, number>();
  const beforeRestart = new Set<string>();
  const restartIdentities = new Set<string>();
  let restartAt = 0;
  let restartReady = 0;
  let tabB = "";

  try {
    await alpha.start();
    expect(new URL(alpha.origin).hostname).toBe(NON_LOOPBACK_HOST);
    gui = await playwright.chromium.launch({
      executablePath: fixture.chrome,
      env: fixture.env,
    });
    const first = await newGuiPage(gui);
    page = first.page;
    const net = first.net;
    observers.push(net);

    await test.step("GUI served by the worker loads from the non-loopback address with no credential error", async () => {
      const html = await (await fetch(`${alpha.origin}/`)).text();
      const bundles = [...html.matchAll(/<script src="([^"]+)"/g)].map(
        (match) => new URL(match[1], `${alpha.origin}/`).toString(),
      );
      expect(bundles.length).toBeGreaterThan(0);
      const sources = await Promise.all(
        bundles.map(async (url) => (await fetch(url)).text()),
      );
      expect(
        sources.some((source) => source.includes("This browser tab is closed")),
        "the worker must serve the GUI built from this checkout",
      ).toBe(true);
      await writeFile(
        info.outputPath("served-gui-bundles.json"),
        JSON.stringify(
          bundles.map((url, index) => ({
            url,
            sha256: createHash("sha256").update(sources[index]).digest("hex"),
          })),
          null,
          2,
        ),
      );
      await page!.goto(`${alpha.origin}/?chat=${chatOne}`);
      await expect(workspaceTab(page!, "Alpha chat one")).toHaveAttribute(
        "aria-selected",
        "true",
        { timeout: 30_000 },
      );
      await expect(composer(page!)).toBeVisible();
      await expectNoCredentialUi(page!);
      expect(net.denied()).toEqual([]);
      await picture(page!, info, "A01-worker-gui-non-loopback");
    });

    await test.step("a chat tool call auto-opens its named Browser tab right of the chat; the newest legacy profile was adopted", async () => {
      const before = site.visits.length;
      await browseAndExpectAutoOpen(page!, site, say.login, {
        title: "Fixture login",
        railText: "Fixture login",
      });
      await writeFile(
        info.outputPath("wire-tool-messages.json"),
        JSON.stringify(await wireToolMessages(alpha.origin, chatOne), null, 2),
      );
      await expect(workspaceTab(page!, "Fixture login")).toBeVisible();
      tabA = await visibleBrowserTab(
        page!,
        net,
        (frame) =>
          frame.url === `${site.origin}/login` &&
          frame.title === "Fixture login",
      );
      await expectBrowserRightOfChat(page!);
      const login = site.lastVisit("/login", before);
      expect(login?.userAgent).toMatch(/Chrome\//);
      expect(login?.cookie).toContain("fixture_legacy=newer");
      expect(login?.cookie).not.toContain("fixture_legacy=older");
      expect(existsSync(newer.path)).toBe(false);
      expect(existsSync(older.path)).toBe(true);
      expect(
        readdirSync(join(fixture.cache, "project_browser_profiles")),
      ).toHaveLength(1);
      expect(
        site.llmCalls.filter((call) => call.responded === "tool_call"),
      ).toHaveLength(1);
      await expectNoCredentialUi(page!);
      await picture(page!, info, "A02-auto-open-right-of-chat");
    });

    await test.step("sign in by clicking and typing inside the live frame", async () => {
      const state = net.frame(tabA);
      if (!state) throw new Error("No login frame");
      const image = frameImage(page!);
      const field = await framePoint(image, state, 91, 87);
      const submit = await framePoint(image, state, 215, 87);
      const startedAt = Date.now();
      await page!.mouse.click(field.x, field.y);
      await page!.keyboard.type("alpha-user");
      await page!.mouse.click(submit.x, submit.y);
      await expect
        .poll(() => site.lastVisit("/signin")?.user, { timeout: 30_000 })
        .toBe("alpha-user");
      await expect
        .poll(() => net.frame(tabA)?.title, { timeout: 30_000 })
        .toBe("Signed in as alpha-user");
      const elapsed = Date.now() - startedAt;
      const window = () =>
        net.requests.filter(
          (request) =>
            (request.startAt >= startedAt || request.endAt === null) &&
            ["input", baseline ? "frame" : "frames/subscribe"].includes(
              request.route,
            ),
        );
      await expect
        .poll(() =>
          window()
            .filter((request) => request.route === "input")
            .every((request) => request.endAt !== null),
        )
        .toBe(true);
      const inputs = window().filter((request) => request.route === "input");
      const frames = window().filter(
        (request) =>
          request.route === (baseline ? "frame" : "frames/subscribe"),
      );
      const overlapping = inputs.map((input) => ({
        input,
        frames: frames.filter(
          (frame) =>
            frame.tab_id === input.tab_id &&
            frame.startAt <= input.startAt &&
            (frame.endAt ?? Infinity) > input.startAt,
        ),
      }));
      await writeFile(
        info.outputPath("login-input-timeline.json"),
        JSON.stringify(
          { elapsed_ms: elapsed, requests: window(), overlapping },
          null,
          1,
        ),
      );
      expect(
        elapsed,
        "click + type + submit must finish within 8 s: one 3 s renderer deadline for the frame showing the result, plus one 3 s input deadline, plus ~2 s for frame polling and the local navigation",
      ).toBeLessThanOrEqual(8_000);
      expect(inputs.filter((input) => input.kind === "click").length).toBe(2);
      expect(inputs.filter((input) => input.kind === "text").length).toBe(
        "alpha-user".length,
      );
      expect(
        inputs.filter((input) => input.status !== 200),
        "every input is accepted",
      ).toEqual([]);
      expect(
        inputs.filter((input) => (input.ms ?? Infinity) >= 3_000),
        "no input reaches the 3 s input deadline",
      ).toEqual([]);
      if (!baseline)
        expect(
          overlapping.some(({ frames: inFlight }) => inFlight.length > 0),
          "an input must overlap the active stream",
        ).toBe(true);
      expect(
        overlapping.filter(
          ({ input, frames: inFlight }) =>
            inFlight.length > 0 && (input.ms ?? Infinity) > 1_500,
        ),
        "an input sent while the frame stream was open must not wait for the stream",
      ).toEqual([]);
      await picture(page!, info, "A03-signed-in-inside-frame");
    });

    await test.step("controlled pane animation and wheel show timely distinct painted frames", async () => {
      const address = browserView(page!).getByRole("textbox", {
        name: "Address",
      });
      await address.fill(`${site.origin}/cadence`);
      await address.press("Enter");
      await expect
        .poll(() => net.frame(tabA)?.url, { timeout: 30_000 })
        .toBe(`${site.origin}/cadence`);
      const state = net.frame(tabA);
      if (!state) throw new Error("No cadence frame");
      await expect(frameImage(page!)).toHaveAttribute(
        "src",
        baseline ? /^data:image\// : /^blob:/,
      );
      await page!.evaluate(
        ({ viewport }) => {
          const image = document.querySelector<HTMLImageElement>(
            'section[aria-label="Browser tab"] img[alt="Browser page"]',
          );
          if (!image) throw new Error("No displayed pane image");
          const canvas = document.createElement("canvas");
          canvas.width = 96;
          canvas.height = 64;
          const context = canvas.getContext("2d", { willReadFrequently: true });
          if (!context) throw new Error("No canvas context");
          const samples: {
            at: number;
            hash: number;
            button: number[];
            stripe: number[];
          }[] = [];
          const start = performance.now();
          const result = { samples, inputAt: 0, start, end: 0 };
          (window as unknown as { __cadence: typeof result }).__cadence =
            result;
          image.addEventListener(
            "mousedown",
            () => {
              result.inputAt = performance.now();
            },
            { once: true, capture: true },
          );
          const sample = () => {
            if (result.inputAt && performance.now() - result.inputAt >= 4500) {
              result.end = performance.now();
              return;
            }
            if (image.complete && image.naturalWidth > 0) {
              context.drawImage(image, 0, 0, 96, 64);
              const pixels = context.getImageData(0, 0, 96, 64).data;
              let hash = 2166136261;
              for (let i = 0; i < pixels.length; i += 4)
                hash = Math.imul(
                  hash ^
                    pixels[i] ^
                    (pixels[i + 1] << 8) ^
                    (pixels[i + 2] << 16),
                  16777619,
                );
              const x = Math.floor((40 / viewport.width) * 96);
              const y = Math.floor((40 / viewport.height) * 64);
              const offset = (y * 96 + x) * 4;
              samples.push({
                at: performance.now(),
                hash: hash >>> 0,
                button: Array.from(pixels.slice(offset, offset + 3)),
                stripe: Array.from(
                  pixels.slice(
                    (Math.floor((350 / viewport.height) * 64) * 96 +
                      Math.floor(
                        ((viewport.width - 30) / viewport.width) * 96,
                      )) *
                      4,
                    (Math.floor((350 / viewport.height) * 64) * 96 +
                      Math.floor(
                        ((viewport.width - 30) / viewport.width) * 96,
                      )) *
                      4 +
                      3,
                  ),
                ),
              });
            }
            requestAnimationFrame(sample);
          };
          requestAnimationFrame(sample);
        },
        { viewport: state.viewport },
      );
      await page!.waitForFunction(
        () =>
          (
            window as unknown as {
              __cadence?: { samples: { button: number[] }[] };
            }
          ).__cadence?.samples.some(
            (sample) => sample.button[0] > sample.button[1] * 1.5,
          ) ?? false,
      );
      const button = await framePoint(frameImage(page!), state, 60, 50);
      await page!.mouse.click(button.x, button.y);
      const middle = await framePoint(frameImage(page!), state, 300, 350);
      await page!.mouse.move(middle.x, middle.y);
      const visits = site.visits.length;
      await page!.mouse.wheel(0, 500);
      await expect
        .poll(() => site.lastVisit("/cadence-scrolled", visits), {
          timeout: 10_000,
        })
        .toBeDefined();
      await page!.waitForFunction(
        () =>
          Boolean(
            (window as unknown as { __cadence?: { end: number } }).__cadence
              ?.end,
          ),
        null,
        { timeout: 10_000 },
      );
      const metric = await page!.evaluate(
        () =>
          (
            window as unknown as {
              __cadence: {
                inputAt: number;
                start: number;
                end: number;
                samples: {
                  at: number;
                  hash: number;
                  button: number[];
                  stripe: number[];
                }[];
              };
            }
          ).__cadence,
      );
      const before = metric.samples
        .filter((sample) => sample.at < metric.inputAt)
        .at(-1);
      const changed = metric.samples.find(
        (sample) =>
          sample.at > metric.inputAt &&
          sample.button[1] > sample.button[0] * 1.5,
      );
      const changes = metric.samples
        .filter(
          (sample, index, samples) =>
            index > 0 &&
            sample.at > metric.inputAt &&
            sample.hash !== samples[index - 1].hash,
        )
        .map((sample) => sample.at - metric.inputAt);
      const distinct = new Set(
        metric.samples
          .filter((sample) => sample.at > metric.inputAt)
          .map((sample) => sample.hash),
      ).size;
      const evidence = {
        definition:
          "inputAt is GUI image mousedown capture; first green button pixel is sampled from the displayed decoded img on animation frames; distinct hashes are downsampled displayed image pixels sampled until 4.5s, including wheel and animation",
        viewport: state.viewport,
        image: { width: state.width, height: state.height },
        inputAt: metric.inputAt,
        baseline: before,
        firstChange: changed,
        finalSample: metric.samples.at(-1),
        latency_ms: changed ? changed.at - metric.inputAt : null,
        distinct,
        visible_change_offsets_ms: changes,
        observed_change_rate_hz:
          changes.length / ((metric.end - metric.inputAt) / 1000),
        samples: metric.samples.length,
      };
      await writeFile(
        info.outputPath("pane-cadence.json"),
        JSON.stringify(evidence, null, 2),
      );
      expect(metric.inputAt).toBeGreaterThan(0);
      expect(before, "sample a red button before input").toBeDefined();
      expect(before!.stripe[2], "unscrolled stripe is blue").toBeGreaterThan(
        before!.stripe[0] * 1.5,
      );
      expect(
        metric.samples.at(-1)!.stripe[0],
        "scroll produces the yellow stripe in the displayed image",
      ).toBeGreaterThan(metric.samples.at(-1)!.stripe[2] * 1.5);
      expect(before!.button[0]).toBeGreaterThan(before!.button[1] * 1.5);
      expect(
        changed,
        "button's green pixels must appear in the displayed image",
      ).toBeDefined();
      expect(changed!.at - metric.inputAt).toBeLessThan(3_000);
      expect(
        distinct,
        "animation and wheel must produce at least four distinct displayed pixel samples (liveness only, not a calibrated throughput threshold)",
      ).toBeGreaterThanOrEqual(4);
      await picture(page!, info, "A03b-cadence");
    });

    await test.step("closing the view, then another tool call reopens it on the signed-in page", async () => {
      const priorStreams = net.requests.filter(
        (request) =>
          request.route === "frames/subscribe" && request.tab_id === tabA,
      );
      if (!baseline) expect(priorStreams.length).toBeGreaterThan(0);
      const activeStream = priorStreams.at(-1)!;
      await page!
        .getByRole("button", { name: "Close Fixture login", exact: true })
        .click();
      await expect(workspaceTab(page!, "Fixture login")).toHaveCount(0);
      await expect(browserView(page!)).toHaveCount(0);
      if (!baseline)
        await expect
          .poll(() => activeStream.endAt, { timeout: 15_000 })
          .not.toBeNull();
      await sendMessage(page!, say.plain);
      await expect(
        page!.getByText("Fixture model reply (plain).", { exact: true }),
      ).toBeVisible({ timeout: 60_000 });
      await expect(workspaceTab(page!, "Fixture login")).toHaveCount(0);
      const before = site.visits.length;
      await browseAndExpectAutoOpen(page!, site, say.check, {
        title: "Fixture login",
        railText: "Fixture login",
      });
      await expect(workspaceTab(page!, "Fixture login")).toBeVisible();
      const reopened = await visibleBrowserTab(
        page!,
        net,
        (frame) =>
          frame.url === `${site.origin}/check` &&
          frame.title === "Signed in as alpha-user",
      );
      expect(reopened).toBe(tabA);
      if (!baseline)
        expect(
          net.requests.filter(
            (request) =>
              request.route === "frames/subscribe" && request.tab_id === tabA,
          ).length,
        ).toBeGreaterThan(priorStreams.length);
      expect(site.lastVisit("/check", before)?.cookie).toContain(
        "fixture_login=alpha-user",
      );
      await expectBrowserRightOfChat(page!);
      await picture(page!, info, "A04-reopened-signed-in");
    });

    await test.step("the user signs in through the Browser view while the agent's chrome call waits for it", async () => {
      await expect(browserView(page!)).toHaveAttribute(
        "data-browser-tab-id",
        tabA,
      );
      const startedAt = Date.now();
      const visits = site.visits.length;
      const calls = () =>
        site.llmCalls.filter((call) => call.script === "waitlogin");
      const frameEvents = () =>
        net.frameLog.filter(
          (frame) => frame.tab_id === tabA && frame.at >= startedAt,
        );
      const visit = (path: string) => site.lastVisit(path, visits);
      const requests = () =>
        net.requests.filter(
          (request) => request.tab_id === tabA && request.startAt >= startedAt,
        );
      try {
        await sendMessage(page!, say.waitLogin);
        await expect
          .poll(() => visit("/wait-started")?.at, { timeout: 15_000 })
          .toBeDefined();
        const loginFrame = () =>
          frameEvents().filter(
            (frame) =>
              frame.at >= visit("/wait-started")!.at &&
              frame.url === `${site.origin}/login?wait=1`,
          );
        await expect.poll(() => loginFrame().length).toBeGreaterThan(0);
        const state = net.frame(tabA);
        if (!state) throw new Error("No login frame during the wait");
        const image = frameImage(page!);
        const field = await framePoint(image, state, 91, 87);
        const submit = await framePoint(image, state, 215, 87);
        expect(calls().map((call) => call.responded)).toEqual(["tool_call"]);
        await page!.mouse.click(field.x, field.y);
        await page!.keyboard.type("alpha-user");
        const typedAt = Date.now();
        await expect
          .poll(() => loginFrame().some((frame) => frame.at > typedAt))
          .toBe(true);
        expect(visit("/check")).toBeUndefined();
        expect(calls()).toHaveLength(1);
        await page!.mouse.click(submit.x, submit.y);
        await expect.poll(() => calls().length, { timeout: 20_000 }).toBe(2);
        const [call, final] = calls();
        expect(final.responded).toBe("final");
        expect(call.toolCallId).toBeTruthy();
        expect(final.resultCallId).toBe(call.toolCallId);
        expect(final.at - visit("/wait-started")!.at).toBeLessThan(
          waitTimeout / 3,
        );
        expect(visit("/signin")?.user).toBe("alpha-user");
        expect(visit("/check")?.cookie).toContain("fixture_login=alpha-user");
        expect(visit("/check")!.at).toBeLessThanOrEqual(final.at);
        expect(JSON.parse(final.toolResult ?? "")).toMatchObject({
          tool: "chrome",
          ok: true,
          url: `${site.origin}/check`,
          steps: [{ ok: true }, { ok: true, summary: "Predicate satisfied" }],
        });
        const inputs = () =>
          requests().filter((request) => request.route === "input");
        await expect
          .poll(() => inputs().map((input) => [input.kind, input.status]))
          .toEqual([
            ["click", 200],
            ...Array.from("alpha-user", () => ["text", 200]),
            ["click", 200],
          ]);
        await expect
          .poll(() => net.frame(tabA)?.url)
          .toBe(`${site.origin}/check`);
        expect(net.frame(tabA)?.title).toBe("Signed in as alpha-user");
      } finally {
        await writeFile(
          info.outputPath("wait-signin-timeline.json"),
          JSON.stringify(
            {
              startedAt,
              waitTimeout,
              calls: calls()
                .map(
                  ({
                    at,
                    responded,
                    toolCallId,
                    resultCallId,
                    toolResult,
                  }) => ({
                    at,
                    responded,
                    toolCallId,
                    resultCallId,
                    excerpt: toolResult?.slice(0, 1500),
                  }),
                )
                .slice(-4),
              visits: site.visits
                .slice(visits)
                .filter((v) =>
                  ["/login", "/wait-started", "/signin", "/check"].includes(
                    v.path,
                  ),
                )
                .map(({ at, path }) => ({ at, path }))
                .slice(-12),
              frames: frameEvents(),
              requests: requests().slice(-128),
            },
            null,
            1,
          ),
        );
      }
      await picture(page!, info, "A04b-signed-in-during-agent-wait");
    });

    await test.step("Browser rail panel lists tabs with their chat; New tab opens next to the chat; Close tab closes Chromium's tab", async () => {
      await railButton(page!).click();
      const panel = dockPanel(page!);
      await expect(panel).toBeVisible();
      const items = panel
        .getByRole("list", { name: "Browser tabs" })
        .locator("li");
      const statusTabs = async () =>
        (
          (await (
            await alpha.request(`${browserEndpoint}status`, {})
          ).json()) as BrowserStatus
        ).tabs;
      const initial = await statusTabs();
      await expect(items).toHaveCount(initial.length, { timeout: 20_000 });
      const own = items.filter({ hasText: "Fixture login" });
      await expect(own).toHaveCount(1);
      await expect(own).toContainText("Alpha chat one");
      await expect(own).toContainText("Open here");
      expect(initial.find((tab) => tab.tab_id === tabA)?.name).toBe(
        "Fixture login",
      );
      await picture(page!, info, "A05-rail-panel");

      await panel.getByRole("button", { name: "New tab", exact: true }).click();
      await expect(items).toHaveCount(initial.length + 1, { timeout: 30_000 });
      await expect
        .poll(() => browserView(page!).getAttribute("data-browser-tab-id"), {
          timeout: 30_000,
        })
        .not.toBe(tabA);
      const created = await browserView(page!).getAttribute(
        "data-browser-tab-id",
      );
      if (!created) throw new Error("New tab view has no tab id");
      closedTabs.add(created);
      await expect
        .poll(() => Boolean(net.frame(created)), { timeout: 30_000 })
        .toBe(true);
      await expectBrowserRightOfChat(page!);
      const withNew = await statusTabs();
      const index = withNew.findIndex((tab) => tab.tab_id === created);
      expect(index).toBeGreaterThanOrEqual(0);
      expect(withNew[index].opened_by_chat).toBe(chatOne);
      await expect(items.nth(index)).toContainText("Alpha chat one");
      await picture(page!, info, "A06-new-tab-next-to-chat");

      closedAt.set(created, Date.now());
      await items
        .nth(index)
        .getByRole("button", { name: /^Close tab: / })
        .click();
      await expect(items).toHaveCount(initial.length, { timeout: 30_000 });
      await expect(
        page!.locator(`[data-browser-tab-id="${created}"]`),
      ).toHaveCount(0);
      const after = (await statusTabs()).map((tab) => tab.tab_id);
      expect(after).not.toContain(created);
      expect(after).toContain(tabA);

      await panel.getByRole("button", { name: "New tab", exact: true }).click();
      await expect(items).toHaveCount(initial.length + 1, { timeout: 30_000 });
      await expect
        .poll(() => browserView(page!).getAttribute("data-browser-tab-id"), {
          timeout: 30_000,
        })
        .not.toBe(tabA);
      const doomed = await browserView(page!).getAttribute(
        "data-browser-tab-id",
      );
      if (!doomed || doomed === created)
        throw new Error("Second new tab view has no fresh tab id");
      closedTabs.add(doomed);
      closedAt.set(doomed, Date.now());
      await alpha.request(`${browserEndpoint}tab`, {
        op: "close",
        tab_id: doomed,
      });
      await expect(
        browserView(page!).getByRole("heading", {
          name: "This browser tab is closed",
        }),
      ).toBeVisible({ timeout: 30_000 });
      await expect(items).toHaveCount(initial.length, { timeout: 30_000 });
      await picture(page!, info, "A07-closed-tab-view");
      await page!
        .getByRole("button", { name: "Close view", exact: true })
        .click();
      await expect(
        page!.locator(`[data-browser-tab-id="${doomed}"]`),
      ).toHaveCount(0);
      await railButton(page!).click();
    });

    await test.step("a second chat's tool call gets its own tab; switching chats swaps the right pane", async () => {
      await openChat(page!, "Alpha chat two");
      await browseAndExpectAutoOpen(page!, site, say.second, {
        title: "Second chat page",
        railText: "Second chat page",
      });
      await expect(workspaceTab(page!, "Second chat page")).toBeVisible();
      tabB = await visibleBrowserTab(
        page!,
        net,
        (frame) => frame.url === `${site.origin}/second`,
      );
      expect(tabB).not.toBe(tabA);
      await expectBrowserRightOfChat(page!);
      const status = (await (
        await alpha.request(`${browserEndpoint}status`, {})
      ).json()) as BrowserStatus;
      const a = status.tabs.find((tab) => tab.tab_id === tabA);
      const b = status.tabs.find((tab) => tab.tab_id === tabB);
      expect(a?.url).toBe(`${site.origin}/check`);
      expect(a?.last_chat).toBe(chatOne);
      expect(b?.last_chat).toBe(chatTwo);
      expect(b?.name).toBe("Second chat page");
      await expect(
        workspaceTab(page!, "Fixture login"),
        "chat one's Browser view must not show in chat two's tab strip",
      ).toHaveCount(0);
      await picture(page!, info, "A08-second-chat-own-tab");

      await openChat(page!, "Alpha chat one");
      await expect(browserView(page!)).toHaveAttribute(
        "data-browser-tab-id",
        tabA,
        { timeout: 20_000 },
      );
      await expectBrowserRightOfChat(page!);
      await expect(workspaceTab(page!, "Fixture login")).toBeVisible();
      await expect(
        workspaceTab(page!, "Second chat page"),
        "chat two's Browser view must not show in chat one's tab strip",
      ).toHaveCount(0);
      await picture(page!, info, "A09-switch-back-to-chat-one");
      await openChat(page!, "Alpha chat two");
      await expect(browserView(page!)).toHaveAttribute(
        "data-browser-tab-id",
        tabB,
        { timeout: 20_000 },
      );
    });

    await test.step("restart the worker: the persisted view shows the closed state, one-click recovery opens a live signed-in tab, and the next tool call stays on it", async () => {
      for (const request of net.requests)
        if (
          request.route === "frames/subscribe" &&
          request.status === 200 &&
          (request.tab_id === tabA || request.tab_id === tabB) &&
          request.runtime_id
        )
          restartIdentities.add(`${request.runtime_id}:${request.tab_id}`);
      restartAt = Date.now();
      timeline.push({
        name: "worker-stop-start",
        at: restartAt,
        local: logClock(restartAt),
      });
      await alpha.stopWorker();
      for (const id of [tabA, tabB]) beforeRestart.add(id);
      await alpha.start(alpha.port);
      restartReady = Date.now();
      timeline.push({
        name: "worker-restart-ready",
        at: restartReady,
        local: logClock(restartReady),
      });
      await page!.goto(`${alpha.origin}/?chat=${chatOne}`);
      await expect(composer(page!)).toBeVisible({ timeout: 30_000 });
      const stale = page!.locator(`[data-browser-tab-id="${tabA}"]`);
      await expect(
        stale,
        "the persisted Browser view of chat one survives the reload",
      ).toBeVisible({ timeout: 30_000 });
      await expect(
        stale.getByRole("heading", { name: "This browser tab is closed" }),
        "a view must not start Chromium by itself",
      ).toBeVisible({ timeout: 30_000 });
      await expect(stale).toContainText("Browser rail");
      const idle = (await (
        await alpha.request(`${browserEndpoint}status`, {})
      ).json()) as BrowserStatus;
      expect(idle.running).toBe(false);
      const chatPane = leafSections(page!).filter({ has: composer(page!) });
      const paneBox = await chatPane.boundingBox();
      if (!paneBox) throw new Error("Chat pane has no bounds");
      await page!.mouse.move(
        paneBox.x + paneBox.width / 2,
        paneBox.y + paneBox.height / 2,
      );
      await expect(workspaceTab(page!, "Alpha chat one")).toHaveAttribute(
        "aria-selected",
        "true",
      );
      await expect(
        page!.getByRole("tab", {
          name: /(^|\s)Alpha chat two$/,
          selected: true,
        }),
      ).toHaveCount(0);
      await picture(page!, info, "A10a-restart-stale-view-closed");

      await stale
        .getByRole("button", { name: "Open browser for this chat" })
        .click();
      await expect(
        stale,
        "one-click recovery replaces the dead view",
      ).toHaveCount(0, { timeout: 30_000 });
      const recovered = await visibleBrowserTab(page!, net, () => true);
      expect(beforeRestart.has(recovered)).toBe(false);
      await expect(browserView(page!).getByRole("alert")).toHaveCount(0);
      await expect(page!.getByRole("alert")).toHaveCount(0);
      const recoveredStatus = (await (
        await alpha.request(`${browserEndpoint}status`, {})
      ).json()) as BrowserStatus;
      expect(recoveredStatus.running).toBe(true);
      expect(
        recoveredStatus.tabs.find((tab) => tab.tab_id === recovered)?.last_chat,
      ).toBe(chatOne);
      const beforeAddress = site.visits.length;
      await browserView(page!)
        .getByRole("textbox", { name: "Address" })
        .fill(`${site.origin}/check`);
      await browserView(page!)
        .getByRole("textbox", { name: "Address" })
        .press("Enter");
      await expect
        .poll(() => net.frame(recovered)?.title, { timeout: 30_000 })
        .toBe("Signed in as alpha-user");
      expect(site.lastVisit("/check", beforeAddress)?.cookie).toContain(
        "fixture_login=alpha-user",
      );
      await expectBrowserRightOfChat(page!);
      await expect(page!.getByRole("alert")).toHaveCount(0);
      await picture(page!, info, "A10b-recovered-signed-in");

      const before = site.visits.length;
      await browseAndExpectAutoOpen(page!, site, say.check, {
        railText: /\/check/,
      });
      const fresh = await visibleBrowserTab(
        page!,
        net,
        (frame) =>
          frame.url === `${site.origin}/check` &&
          frame.title === "Signed in as alpha-user",
      );
      expect(fresh, "the chat's next tool call stays on its own tab").toBe(
        recovered,
      );
      expect(site.lastVisit("/check", before)?.cookie).toContain(
        "fixture_login=alpha-user",
      );
      await expectBrowserRightOfChat(page!);
      await picture(page!, info, "A10-after-worker-restart");
    });

    await test.step("a second project's separate worker is not signed in", async () => {
      await beta.start();
      const other = await newGuiPage(gui!);
      page = other.page;
      observers.push(other.net);
      await page.goto(`${beta.origin}/?chat=${betaChat}`);
      await expect(composer(page)).toBeVisible({ timeout: 30_000 });
      const before = site.visits.length;
      await browseAndExpectAutoOpen(page, site, say.check, {
        railText: /\/check/,
      });
      await visibleBrowserTab(
        page,
        other.net,
        (frame) =>
          frame.url === `${site.origin}/check` && frame.title === "Signed out",
      );
      const visit = site.lastVisit("/check", before);
      expect(visit?.cookie).not.toContain("fixture_login");
      expect(visit?.cookie).not.toContain("fixture_legacy");
      await picture(page, info, "A11-project-beta-isolated");
    });

    for (const net of observers) {
      expect(net.denied()).toEqual([]);
      expect(
        net.streamErrors.filter(
          (failure) =>
            !(
              (failure.status === 410 && closedTabs.has(failure.tab_id)) ||
              ([404, 410].includes(failure.status ?? 0) &&
                beforeRestart.has(failure.tab_id))
            ),
        ),
      ).toEqual([]);
      if (!baseline)
        expect(
          net.requests.filter((request) => request.route === "frame"),
          "the pane must not poll /frame",
        ).toEqual([]);
      if (!baseline)
        await expect
          .poll(
            () =>
              invalidSubscribeAttempts(net.requests, closedAt, {
                at: restartAt,
                ready: restartReady,
                identities: restartIdentities,
              }),
            {
              message:
                "every subscription attempt must receive HTTP 200 or have an explicitly expected terminal outcome",
            },
          )
          .toEqual([]);
      await settledFailures(net);
      recordTransientConflicts(info, [net]);
      expect(
        net.failures.filter((failure) => {
          const tabId =
            (JSON.parse(failure.request ?? "{}") as { tab_id?: string })
              .tab_id ?? "";
          return !(
            (failure.status === 410 && closedTabs.has(tabId)) ||
            ([404, 410].includes(failure.status) && beforeRestart.has(tabId)) ||
            isTransientRendererConflict(failure)
          );
        }),
        "Browser routes may only refuse tabs this test closed (410), tabs of the browser before the worker restart (404 not running / 410), or answer a transient renderer 409 on /frame or /resize",
      ).toEqual([]);
    }
  } catch (error) {
    if (page && !page.isClosed()) {
      await picture(page, info, "failure-gui");
      await writeFile(
        info.outputPath("failure-gui-storage.json"),
        JSON.stringify(
          await page.evaluate(() => ({
            local: { ...localStorage },
            session: { ...sessionStorage },
          })),
          null,
          1,
        ),
      );
      await writeFile(
        info.outputPath("failure-gui-aria.yaml"),
        await page.locator("body").ariaSnapshot(),
      );
    }
    throw error;
  } finally {
    await writeRouteEvidence(info, observers);
    await keepDiagnostics(
      info,
      fixture,
      site,
      {
        "alpha-worker": alpha.diagnostics(),
        "beta-worker": beta.diagnostics(),
      },
      [alphaRoot, betaRoot],
    );
    await closeAll(gui);
    try {
      await alpha.stopWorker();
      await beta.stopWorker();
    } finally {
      await site.close();
    }
  }
});

for (const authEnabled of [false, true]) {
  test(`daemon with auth ${
    authEnabled ? "ON (normal UI login)" : "OFF"
  }: a chat tool call opens the Browser split and every Browser route answers`, async ({
    playwright,
  }, info) => {
    test.setTimeout(420_000);
    const fixture = fixtureEnvironment();
    const site = await startSite();
    await writeEngineConfig(fixture, site.origin);
    const say = scripts(site);
    const lane = authEnabled ? "daemon-auth-on" : "daemon-auth-off";
    const daemon = new Daemon(fixture, `${lane}-dir`);
    const root = await createProjectRoot(fixture, lane);
    const chat = await writeChat(root, `${lane} chat`);
    let gui: Browser | undefined;
    let page: Page | undefined;
    const laneObservers: Observer[] = [];
    try {
      await daemon.start({ authEnabled });
      const project = await daemon.openProject(root);
      gui = await playwright.chromium.launch({
        executablePath: fixture.chrome,
        env: fixture.env,
      });
      const opened = await newGuiPage(gui);
      page = opened.page;
      const net = opened.net;
      laneObservers.push(net);
      if (authEnabled)
        await opened.context.addCookies([
          await daemon.loginCookie(project.project_id),
        ]);
      const base = `/p/${project.project_id}`;
      await page.goto(`${daemon.origin}${base}/?chat=${chat}`);
      await expect(composer(page)).toBeVisible({ timeout: 30_000 });
      await railButton(page).click();
      await expect(
        dockPanel(page).getByRole("button", { name: "Start browser" }),
        "the Browser dock offers Start instead of launching Chromium itself",
      ).toBeVisible({ timeout: 20_000 });
      const idle = await inPagePost(page, `${base}${browserEndpoint}status`);
      expect(idle.status).toBe(200);
      expect((JSON.parse(idle.body) as BrowserStatus).running).toBe(false);
      await picture(
        page,
        info,
        `${authEnabled ? "C" : "B"}00-${lane}-dock-start-button`,
      );
      await railButton(page).click();
      await browseAndExpectAutoOpen(page, site, say.login, {
        title: "Fixture login",
        railText: "Fixture login",
      });
      await expect(workspaceTab(page, "Fixture login")).toBeVisible();
      await visibleBrowserTab(
        page,
        net,
        (frame) =>
          frame.url === `${site.origin}/login` &&
          frame.title === "Fixture login",
      );
      await expectBrowserRightOfChat(page);
      const start = await inPagePost(page, `${base}${browserEndpoint}start`);
      expect(start.status).toBe(200);
      const status = await inPagePost(page, `${base}${browserEndpoint}status`);
      expect(status.status).toBe(200);
      expect((JSON.parse(status.body) as BrowserStatus).running).toBe(true);
      expect(net.streamErrors).toEqual([]);
      if (!baseline)
        expect(
          net.requests.filter((request) => request.route === "frame"),
          "the pane must not poll /frame",
        ).toEqual([]);
      const routes = net.browserResponses();
      expect(routes.filter(({ status }) => status === 401)).toEqual([]);
      if (!baseline)
        await expect
          .poll(() => invalidSubscribeAttempts(net.requests), {
            message:
              "every subscription attempt must receive HTTP 200 or abort without a response",
          })
          .toEqual([]);
      await settledFailures(net);
      recordTransientConflicts(info, [net]);
      expect(
        net.failures.filter((failure) => !isTransientRendererConflict(failure)),
        "only transient renderer 409s on /frame or /resize are allowed",
      ).toEqual([]);
      for (const route of ["status", baseline ? "frame" : "frames/subscribe"])
        expect(
          routes.some(
            ({ path, status }) =>
              path === `${base}${browserEndpoint}${route}` && status === 200,
          ),
        ).toBe(true);
      await expectNoCredentialUi(page);
      if (!authEnabled) {
        const bare = await fetch(
          `${daemon.origin}${base}${browserEndpoint}status`,
          {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: "{}",
          },
        );
        expect(bare.status).toBe(200);
      }
      await picture(page, info, `${authEnabled ? "C" : "B"}01-${lane}`);
    } catch (error) {
      if (page && !page.isClosed()) await picture(page, info, "failure-gui");
      throw error;
    } finally {
      await writeRouteEvidence(info, laneObservers);
      await keepDiagnostics(
        info,
        fixture,
        site,
        {
          daemon: daemon.diagnostics(),
        },
        [root],
      );
      await closeAll(gui);
      try {
        await daemon.stopDaemon();
      } finally {
        await site.close();
      }
    }
  });
}
