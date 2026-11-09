import { writeFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";

const ROUTE = "/tests/e2e/apps-sandbox.html";

type Harness = {
  mount: (a?: "light" | "dark") => Promise<void>;
  generation: number;
  channel: string | null;
  rejections: { reason: string; data: unknown }[];
  calls: { method: string; params: unknown }[];
  notify: (m: string, p: unknown) => boolean;
  capture: (p?: unknown) => Promise<Record<string, unknown>>;
  rawCapture: (p?: unknown) => Promise<unknown>;
  postFromHost: (d: unknown) => void;
  samplePng: (
    url: string,
    pts: [number, number][],
  ) => Promise<{ width: number; height: number; pixels: number[][] }>;
};
declare global {
  interface Window {
    __apps: Harness;
  }
}

async function open(page: Page) {
  await page.goto(ROUTE);
  await expect(page.locator("body[data-ready='1']")).toBeAttached();
  return page.frameLocator("iframe[title=app]");
}

function appFrame(page: Page) {
  const f = page.frames().find((fr) => fr !== page.mainFrame());
  if (!f) throw new Error("no app frame");
  return f;
}

test("sandbox is opaque-origin and CSP blocks network, eval and parent access", async ({
  page,
}) => {
  await open(page);
  const iframe = page.locator("iframe[title=app]");
  await expect(iframe).toHaveAttribute("sandbox", "allow-scripts");
  const f = appFrame(page);
  await expect
    .poll(() =>
      f.evaluate(
        () => (window as unknown as { __fetchResult: string }).__fetchResult,
      ),
    )
    .toMatch(/^blocked/);
  expect(
    await f.evaluate(
      () => (window as unknown as { __evalResult: string }).__evalResult,
    ),
  ).toMatch(/^blocked/);
  expect(
    await f.evaluate(
      () => (window as unknown as { __parentAccess: string }).__parentAccess,
    ),
  ).toMatch(/^blocked/);
  expect(await f.evaluate(() => origin)).toBe("null");
  expect(
    await f.evaluate(() =>
      Object.isFrozen((window as unknown as { refactApp: object }).refactApp),
    ),
  ).toBe(true);
});

test("SDK round-trips state, events, context staging and host notifications", async ({
  page,
}) => {
  await open(page);
  const f = appFrame(page);
  const state = await f.evaluate(() =>
    (
      window as unknown as { refactApp: { getState: () => Promise<unknown> } }
    ).refactApp.getState(),
  );
  expect(state).toEqual({ count: 1 });
  await f.evaluate(async () => {
    const a = (
      window as unknown as {
        refactApp: Record<string, (...x: unknown[]) => Promise<unknown>>;
      }
    ).refactApp;
    await a.setState({ count: 2 });
    await a.emit("clicked", { x: 1 });
    await a.stageContext("hello", "lbl");
  });
  const calls = await page.evaluate(() => window.__apps.calls);
  expect(calls.map((c) => c.method)).toEqual([
    "state.set",
    "event.emit",
    "context.stage",
  ]);
  const denied = await f.evaluate(() =>
    (
      window as unknown as {
        refactApp: { requestAction: (n: string) => Promise<unknown> };
      }
    ).refactApp
      .requestAction("deploy")
      .then(
        () => "ok",
        (e: { code?: string }) => e.code,
      ),
  );
  expect(denied).toBe("unsupported");
  await page.evaluate(() => {
    window.__apps.notify("theme", { appearance: "dark" });
    window.__apps.notify("data", { name: "feed", payload: [1, 2] });
  });
  await expect
    .poll(() =>
      f.evaluate(
        () => (window as unknown as { __events: unknown[] }).__events.length,
      ),
    )
    .toBe(2);
  expect(
    await f.evaluate(() =>
      document.documentElement.getAttribute("data-refact-appearance"),
    ),
  ).toBe("dark");
});

test("bridge rejects stale channel, stale generation and non-frame sources", async ({
  page,
}) => {
  await open(page);
  const f = appFrame(page);
  const oldChannel = await page.evaluate(() => window.__apps.channel);
  // forged message from parent window itself (wrong source)
  await page.evaluate((channel) => {
    window.__apps.postFromHost({
      type: "refact-app",
      v: 1,
      channel,
      generation: window.__apps.generation,
      kind: "request",
      id: "c900",
      method: "context.stage",
      params: { text: "forged" },
    });
  }, oldChannel);
  // wrong channel from frame
  await f.evaluate(() =>
    parent.postMessage(
      {
        type: "refact-app",
        v: 1,
        channel: "deadbeefdeadbeef",
        generation: 1,
        kind: "request",
        id: "c901",
        method: "context.stage",
        params: { text: "x" },
      },
      "*",
    ),
  );
  // wrong generation from frame with right channel
  await f.evaluate(
    (channel) =>
      parent.postMessage(
        {
          type: "refact-app",
          v: 1,
          channel,
          generation: 99,
          kind: "request",
          id: "c902",
          method: "context.stage",
          params: { text: "x" },
        },
        "*",
      ),
    oldChannel,
  );
  await expect
    .poll(() =>
      page.evaluate(() => window.__apps.rejections.map((r) => r.reason)),
    )
    .toEqual(["envelope", "envelope"]);
  // remount: old bound channel is now stale
  await page.evaluate(() => window.__apps.mount());
  const f2 = appFrame(page);
  await f2.evaluate(
    (channel) =>
      parent.postMessage(
        {
          type: "refact-app",
          v: 1,
          channel,
          generation: 1,
          kind: "request",
          id: "c903",
          method: "context.stage",
          params: { text: "stale" },
        },
        "*",
      ),
    oldChannel,
  );
  await expect
    .poll(() => page.evaluate(() => window.__apps.rejections.length))
    .toBe(3);
  const staged = await page.evaluate(() =>
    window.__apps.calls.filter((c) => c.method === "context.stage"),
  );
  expect(staged).toEqual([]);
  expect(await page.evaluate(() => window.__apps.generation)).toBe(2);
});

test("current-document capture produces bounded PNG reflecting live state with masks", async ({
  page,
}) => {
  await open(page);
  const f = appFrame(page);
  await f.evaluate(() => {
    const s = document.getElementById("scroller") as HTMLElement;
    s.scrollTop = 100;
    (document.getElementById("text") as HTMLInputElement).value = "typed live";
    (document.getElementById("check") as HTMLInputElement).checked = true;
    (document.getElementById("sel") as HTMLSelectElement).value = "b";
    (document.getElementById("area") as HTMLTextAreaElement).value =
      "area text";
  });
  const loadsBefore = await page.evaluate(
    () => performance.getEntriesByType("navigation").length,
  );
  const t0 = Date.now();
  const res = await page.evaluate(() =>
    window.__apps.capture({ maxWidth: 400, maxHeight: 300 }),
  );
  const elapsed = Date.now() - t0;
  expect(res.status, String(res.reason)).toBe("ok");
  expect(res.format).toBe("image/png");
  expect(res.method).toBe("dom-raster");
  expect(res.trust).toBe("app-reported");
  expect(res.width).toBe(400);
  expect(res.height).toBe(300);
  expect(res.masks).toBe(2);
  expect(res.unsupported).toContain("canvas-webgl");
  expect(res.partial).toBe(true);
  expect(String(res.dataUrl)).toMatch(/^data:image\/png;base64,iVBORw0KGgo/);
  const sample = await page.evaluate(
    (url) =>
      window.__apps.samplePng(url, [
        [60, 30], // scroller after scrollTop=100 -> blue row
        [60, 80], // -> yellow row
        [200, 40], // 2d canvas cyan
        [60, 262], // password masked
        [200, 262], // sensitive masked
      ]),
    String(res.dataUrl),
  );
  expect(sample.width).toBe(400);
  const [blue, yellow, cyan, pw, secret] = sample.pixels;
  expect(blue.slice(0, 3)).toEqual([0, 0, 255]);
  expect(yellow.slice(0, 3)).toEqual([255, 255, 0]);
  expect(cyan.slice(0, 3)).toEqual([0, 255, 255]);
  expect(pw.slice(0, 3)).not.toEqual([255, 0, 255]);
  expect(secret.slice(0, 3)).not.toEqual([255, 0, 255]);
  // live state survives: no reload/navigation occurred
  expect(
    await f.evaluate(
      () => (document.getElementById("text") as HTMLInputElement).value,
    ),
  ).toBe("typed live");
  expect(
    await page.evaluate(
      () => performance.getEntriesByType("navigation").length,
    ),
  ).toBe(loadsBefore);
  expect(await page.evaluate(() => window.__apps.generation)).toBe(1);
  test.info().annotations.push({
    type: "capture",
    description: JSON.stringify({
      ...res,
      dataUrl: `<${String(res.bytes)} bytes>`,
      elapsedMs: elapsed,
    }),
  });
  const png = Buffer.from(String(res.dataUrl).split(",")[1], "base64");
  if (process.env.APPS_CAPTURE_OUT) {
    writeFileSync(process.env.APPS_CAPTURE_OUT, png);
  }
  await test
    .info()
    .attach("apps-capture.png", { body: png, contentType: "image/png" });
});

test("capture respects size caps and reports unsupported instead of oversized output", async ({
  page,
}) => {
  await open(page);
  const small = await page.evaluate(() =>
    window.__apps.capture({ maxWidth: 100, maxHeight: 100 }),
  );
  expect(small.status).toBe("ok");
  expect(small.width).toBeLessThanOrEqual(100);
  expect(small.height).toBeLessThanOrEqual(100);
  const tiny = await page.evaluate(() =>
    window.__apps.capture({ maxBytes: 1024, maxWidth: 400, maxHeight: 300 }),
  );
  expect(tiny.status).toBe("unsupported");
  expect(tiny.dataUrl).toBeUndefined();
});

test("whole-document sensitive marker refuses capture", async ({ page }) => {
  await open(page);
  const f = appFrame(page);
  await f.evaluate(() => document.body.setAttribute("data-sensitive", ""));
  const res = await page.evaluate(() => window.__apps.capture());
  expect(res.status).toBe("unsupported");
  expect(res.dataUrl).toBeUndefined();
});
