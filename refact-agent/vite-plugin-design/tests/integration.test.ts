import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import {
  createServer as createHttpServer,
  get as httpGet,
  type Server,
} from "node:http";
import os from "node:os";
import path from "node:path";
import type { AddressInfo } from "node:net";
import WebSocket from "ws";

import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { createServer, type ViteDevServer } from "vite";

import { parseDesignSurfaceMessage } from "../../gui/src/features/Design/surfaceContract";
import refactDesign from "../src/index";

const chromePath = process.env.CHROME_PATH ?? "/usr/bin/google-chrome";
const e2eEnabled = process.env.REFACT_DESIGN_E2E === "1";
const browserAvailable = existsSync(chromePath);
const runE2e = e2eEnabled && browserAvailable;

if (!e2eEnabled) {
  console.info(
    "Skipping Design iframe integration: set REFACT_DESIGN_E2E=1 to run Chrome.",
  );
} else if (!browserAvailable) {
  console.info(
    `Skipping Design iframe integration: Chrome was not found at ${chromePath}.`,
  );
}

type CdpResult = Record<string, unknown>;
type CdpEvent = { method?: string; params?: Record<string, unknown> };

class CdpClient {
  private nextId = 1;
  private readonly pending = new Map<
    number,
    { resolve: (value: CdpResult) => void; reject: (error: Error) => void }
  >();
  private readonly events = new Map<string, CdpEvent[]>();

  private constructor(private readonly socket: WebSocket) {
    socket.on("message", (data) => {
      const message = JSON.parse(String(data)) as CdpEvent & {
        id?: number;
        result?: CdpResult;
        error?: { message?: string };
      };
      if (message.id !== undefined) {
        const callback = this.pending.get(message.id);
        if (!callback) return;
        this.pending.delete(message.id);
        if (message.error) {
          callback.reject(
            new Error(
              message.error.message ?? "Chrome DevTools command failed",
            ),
          );
        } else {
          callback.resolve(message.result ?? {});
        }
        return;
      }
      if (message.method) {
        const queue = this.events.get(message.method) ?? [];
        queue.push(message);
        this.events.set(message.method, queue);
      }
    });
  }

  static async connect(url: string): Promise<CdpClient> {
    const socket = new WebSocket(url);
    await new Promise<void>((resolve, reject) => {
      socket.once("open", () => resolve());
      socket.once("error", () =>
        reject(new Error("Chrome DevTools connection failed")),
      );
    });
    return new CdpClient(socket);
  }

  send(
    method: string,
    params: Record<string, unknown> = {},
  ): Promise<CdpResult> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async waitForEvent(method: string): Promise<CdpEvent> {
    await waitFor(() => (this.events.get(method)?.length ?? 0) > 0);
    const event = this.events.get(method)?.shift();
    if (!event) throw new Error(`Missing ${method} event`);
    return event;
  }

  close(): void {
    this.socket.close();
  }
}

async function waitFor(
  predicate: () => boolean | Promise<boolean>,
  timeoutMs = 10_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(`Condition was not met within ${timeoutMs}ms`);
}

async function waitForFile(filePath: string): Promise<void> {
  await waitFor(() => existsSync(filePath));
}

async function listen(server: Server): Promise<string> {
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address() as AddressInfo;
  return `http://127.0.0.1:${address.port}`;
}

async function getJson<T>(url: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    httpGet(url, (response) => {
      const chunks: Buffer[] = [];
      response.on("data", (chunk: Buffer) => chunks.push(chunk));
      response.on("end", () => {
        try {
          resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")) as T);
        } catch (error) {
          reject(error instanceof Error ? error : new Error(String(error)));
        }
      });
    }).on("error", reject);
  });
}

async function evaluate(
  client: CdpClient,
  expression: string,
): Promise<unknown> {
  const result = await client.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  const remote = result.result as
    | { value?: unknown; description?: string }
    | undefined;
  if (!remote) throw new Error("Chrome returned no evaluation result");
  return remote.value;
}

async function launchChrome(
  profile: string,
): Promise<{ process: ChildProcess; client: CdpClient }> {
  const process = spawn(
    chromePath,
    [
      "--headless=new",
      "--no-sandbox",
      "--disable-gpu",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "about:blank",
    ],
    { stdio: "ignore" },
  );
  const portFile = path.join(profile, "DevToolsActivePort");
  await waitForFile(portFile);
  const [port] = (await readFile(portFile, "utf8")).trim().split("\n");
  if (!port) throw new Error("Chrome did not report its DevTools port");
  await waitFor(async () => {
    const targets = await getJson<
      Array<{ type?: string; webSocketDebuggerUrl?: string }>
    >(`http://127.0.0.1:${port}/json/list`);
    return targets.some((candidate) => candidate.type === "page");
  });
  const targets = await getJson<
    Array<{ type?: string; webSocketDebuggerUrl?: string }>
  >(`http://127.0.0.1:${port}/json/list`);
  const target = targets.find((candidate) => candidate.type === "page");
  if (!target?.webSocketDebuggerUrl)
    throw new Error("Chrome did not expose a page target");
  return {
    process,
    client: await CdpClient.connect(target.webSocketDebuggerUrl),
  };
}

describe.skipIf(!runE2e)("Design cross-origin iframe", () => {
  let directory: string;
  let parentServer: Server;
  let parentOrigin: string;
  let childServer: ViteDevServer;
  let childOrigin: string;
  let chrome: ChildProcess;
  let client: CdpClient;

  beforeAll(async () => {
    directory = await mkdtemp(
      path.join(os.tmpdir(), "refact-design-integration-"),
    );
    let currentChildOrigin = "";
    parentServer = createHttpServer((_request, response) => {
      response.setHeader("Content-Type", "text/html; charset=utf-8");
      response.end(`<!doctype html>
        <iframe id="design" src="${currentChildOrigin}" style="border:0;width:500px;height:300px"></iframe>
        <script>
          window.__designMessages = [];
          const frame = document.querySelector('#design');
          const childOrigin = ${JSON.stringify(currentChildOrigin)};
          window.addEventListener('message', (event) => {
            if (event.origin !== childOrigin || event.source !== frame.contentWindow) return;
            window.__designMessages.push(event.data);
          });
          frame.addEventListener('load', () => {
            frame.contentWindow.postMessage({
              type: 'refact:set-state',
              payload: { theme: 'dark', pickerEnabled: true, devicePixelRatio: 1 }
            }, childOrigin);
          });
          window.sendDesignTool = (name, args) => frame.contentWindow.postMessage({
            type: 'refact:call-tool', payload: { name, arguments: args }
          }, childOrigin);
        </script>`);
    });
    parentOrigin = await listen(parentServer);

    await writeFile(
      path.join(directory, "index.html"),
      '<script type="module" src="/src.tsx"></script>',
    );
    await writeFile(
      path.join(directory, "src.tsx"),
      `function h(tag, props, ...children) {
        const element = document.createElement(tag);
        for (const [name, value] of Object.entries(props ?? {})) {
          if (name === "className") element.className = String(value);
          else element.setAttribute(name, String(value));
        }
        element.append(...children);
        return element;
      }
      const target = <button id="target">Design target</button>;
      target.style.margin = "40px";
      target.style.width = "160px";
      target.style.height = "60px";
      document.body.append(target);`,
    );
    childServer = await createServer({
      root: directory,
      logLevel: "silent",
      esbuild: { jsxFactory: "h", jsxFragment: "Fragment" },
      resolve: {
        alias: {
          "@refact/vite-plugin-design/runtime": path.resolve("src/runtime.ts"),
        },
      },
      plugins: [refactDesign({ allowedParentOrigins: [parentOrigin] })],
      server: { host: "127.0.0.1", port: 0 },
    });
    await childServer.listen();
    const childAddress = childServer.httpServer?.address() as AddressInfo;
    childOrigin = `http://127.0.0.1:${childAddress.port}`;
    currentChildOrigin = childOrigin;

    const launched = await launchChrome(path.join(directory, "chrome-profile"));
    chrome = launched.process;
    client = launched.client;
    await client.send("Page.enable");
    await client.send("Runtime.enable");
    await client.send("Page.navigate", { url: parentOrigin });
    await waitFor(
      async () =>
        (await evaluate(client, "document.readyState")) === "complete",
    );
  }, 30_000);

  afterAll(async () => {
    client?.close();
    if (chrome) {
      chrome.kill();
      await new Promise<void>((resolve) =>
        chrome.once("exit", () => resolve()),
      );
    }
    await childServer?.close();
    await new Promise<void>((resolve) => parentServer?.close(() => resolve()));
    if (directory) await rm(directory, { recursive: true, force: true });
  });

  it("delivers selection and one validated Apply with source location", async () => {
    await waitFor(async () => {
      const messages = (await evaluate(
        client,
        "window.__designMessages",
      )) as unknown[];
      return messages.some(
        (message) =>
          parseDesignSurfaceMessage(message)?.type === "refact:design-ready",
      );
    });

    const points = (await evaluate(
      client,
      `Promise.all([
        Promise.resolve(document.querySelector('#design').getBoundingClientRect()).then(r => ({ x: r.x, y: r.y })),
        new Promise(resolve => {
          const frame = document.querySelector('#design');
          const receive = event => {
            if (event.source !== frame.contentWindow || event.data?.type !== 'refact:element-selected') return;
            window.removeEventListener('message', receive);
            resolve(event.data.payload.rect);
          };
          window.addEventListener('message', receive);
          frame.contentWindow.postMessage({ type: 'refact:call-tool', payload: { name: 'design.add-annotation', arguments: { id: 'probe', selector: '#target', label: 'probe' } } }, ${JSON.stringify(
            childOrigin,
          )});
          setTimeout(() => resolve({ x: 40, y: 40, width: 160, height: 60 }), 100);
        })
      ])`,
    )) as [
      { x: number; y: number },
      { x: number; y: number; width: number; height: number },
    ];
    const clickX = points[0].x + points[1].x + points[1].width / 2;
    const clickY = points[0].y + points[1].y + points[1].height / 2;
    await client.send("Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: clickX,
      y: clickY,
    });
    await client.send("Input.dispatchMouseEvent", {
      type: "mousePressed",
      button: "left",
      clickCount: 1,
      x: clickX,
      y: clickY,
    });
    await client.send("Input.dispatchMouseEvent", {
      type: "mouseReleased",
      button: "left",
      clickCount: 1,
      x: clickX,
      y: clickY,
    });

    await waitFor(async () => {
      const messages = (await evaluate(
        client,
        "window.__designMessages",
      )) as unknown[];
      return messages.some(
        (message) =>
          parseDesignSurfaceMessage(message)?.type ===
          "refact:element-selected",
      );
    });
    const rawMessages = (await evaluate(
      client,
      "window.__designMessages",
    )) as unknown[];
    const selection = rawMessages
      .map(parseDesignSurfaceMessage)
      .find((message) => message?.type === "refact:element-selected");
    expect(selection?.type).toBe("refact:element-selected");
    if (selection?.type !== "refact:element-selected")
      throw new Error("Missing selection");
    expect(selection.payload.sourceFile).toBe("src.tsx");
    expect(selection.payload.line).toBeGreaterThan(0);

    await evaluate(
      client,
      `window.sendDesignTool('design.apply-style-edit', {
        selector: ${JSON.stringify(selection.payload.selector)},
        styles: { color: 'rgb(255, 0, 0)' }
      });
      window.sendDesignTool('design.apply', { instruction: 'Use the selected source', screenshot: null });`,
    );
    await waitFor(async () => {
      const messages = (await evaluate(
        client,
        "window.__designMessages",
      )) as unknown[];
      return messages.some(
        (message) =>
          parseDesignSurfaceMessage(message)?.type ===
          "refact:send-followup-turn",
      );
    });

    const finalMessages = (
      (await evaluate(client, "window.__designMessages")) as unknown[]
    )
      .map(parseDesignSurfaceMessage)
      .filter((message) => message?.type === "refact:send-followup-turn");
    expect(finalMessages).toHaveLength(1);
    const apply = finalMessages[0];
    if (apply?.type !== "refact:send-followup-turn")
      throw new Error("Missing Apply");
    const content = JSON.parse(apply.payload.content) as {
      edits: Array<{ selector: string; styles: Record<string, string> }>;
      instruction: string;
      screenshot: string | null;
    };
    expect(content.edits).toEqual([
      {
        selector: selection.payload.selector,
        styles: { color: "rgb(255, 0, 0)" },
      },
    ]);
    expect(content.edits[0]?.selector).toContain("data-refact-src");
    expect(content.edits[0]?.selector).toContain("src.tsx:");
    expect(content.instruction).toBe("Use the selected source");
    expect(content.screenshot).toBeNull();
  }, 30_000);
});
