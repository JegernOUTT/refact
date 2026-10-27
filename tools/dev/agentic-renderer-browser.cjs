const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const http = require("node:http");
const { execFileSync } = require("node:child_process");
const { pathToFileURL } = require("node:url");

const root = path.resolve(__dirname, "../..");
const gui = path.join(root, "refact-agent/gui");
const candidate = "76c2a19a1c205664807afe3c3dea60beb1c624c2";
const candidateTree = "f9ed2fb78ec8898feb4cb9aeac639be382e52dfb";
const fixture = "/tests/e2e/agentic-renderer-qualification.html";
const markdown = `# Complete report\n\n${"Evidence preserved. ".repeat(
  80,
)}\n\n[Source reference](#report-source)\n\nEND OF REPORT`;
const files = [
  "tools/dev/agentic-renderer-browser.cjs",
  "refact-agent/gui/tests/e2e/agentic-renderer-qualification.html",
  "refact-agent/gui/tests/e2e/agentic-renderer-qualification.tsx",
];
const cssClient = `
const sheets = new Map();
export function updateStyle(id, content) {
  let style = sheets.get(id);
  if (!style) {
    style = document.createElement("style");
    style.setAttribute("data-vite-dev-id", id);
    document.head.appendChild(style);
    sheets.set(id, style);
  }
  style.textContent = content;
}
export function removeStyle(id) {
  sheets.get(id)?.remove();
  sheets.delete(id);
}
export function createHotContext() {
  return { accept() {}, prune() {}, dispose() {}, on() {}, off() {}, data: {} };
}
`;
const git = (...args) =>
  execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();

function containment() {
  const namespace = fs.readlinkSync("/proc/self/ns/net");
  assert.equal(namespace, process.env.REFACT_OFFLINE_NETNS);
  assert.ok(process.env.REFACT_OFFLINE_HOST_NETNS);
  assert.notEqual(namespace, process.env.REFACT_OFFLINE_HOST_NETNS);
  const status = Object.fromEntries(
    fs
      .readFileSync("/proc/self/status", "utf8")
      .split("\n")
      .filter((line) => line.includes(":"))
      .map((line) => line.split(/:\s*/)),
  );
  const caps = Object.fromEntries(
    ["CapEff", "CapPrm", "CapInh", "CapAmb", "CapBnd"].map((key) => {
      assert.equal(BigInt(`0x${status[key]}`), 0n);
      return [key, status[key]];
    }),
  );
  assert.equal(status.NoNewPrivs, "1");
  const interfaces = fs
    .readFileSync("/proc/net/dev", "utf8")
    .split("\n")
    .filter((line) => line.includes(":"))
    .map((line) => line.split(":")[0].trim());
  assert.deepEqual(interfaces, ["lo"]);
  assert.equal(
    fs.readFileSync("/proc/net/route", "utf8").trim().split("\n").length,
    1,
  );
  assert.ok(!process.env.NODE_OPTIONS);
  assert.equal(process.execArgv.length, 0);
  const removed = [];
  for (const key of Object.keys(process.env)) {
    if (
      !/^(PATH|HOME|USER|LOGNAME|LANG|LC_ALL|TZ|REFACT_OFFLINE_NETNS|REFACT_OFFLINE_HOST_NETNS|REFACT_RUN_LIVE_CHAT_TESTS)$/.test(
        key,
      )
    ) {
      removed.push(key);
      delete process.env[key];
    }
  }
  return {
    namespace,
    hostNamespace: process.env.REFACT_OFFLINE_HOST_NETNS,
    caps,
    noNewPrivs: 1,
    removed,
  };
}

function source(expected) {
  assert.equal(git("rev-parse", `${candidate}^{tree}`), candidateTree);
  git("merge-base", "--is-ancestor", candidate, "HEAD");
  const head = git("rev-parse", "--verify", "HEAD^{commit}");
  const tree = git("rev-parse", "--verify", "HEAD^{tree}");
  const changed = git("diff", "--name-only", candidate)
    .split("\n")
    .filter(Boolean);
  const untracked = git("ls-files", "--others", "--exclude-standard")
    .split("\n")
    .filter(Boolean);
  if (expected) {
    assert.equal(head, expected.head);
    assert.equal(tree, expected.tree);
    assert.equal(git("status", "--porcelain=v1", "--untracked-files=all"), "");
  } else {
    assert.ok([...changed, ...untracked].every((file) => files.includes(file)));
  }
  return {
    head,
    tree,
    candidate,
    candidateTree,
    receiptMode: expected ? "exact-head-tree" : "baseline-harness-delta",
    expected: expected ?? null,
    immutableSources: expected
      ? git("ls-tree", "-r", "--full-tree", "HEAD").split("\n")
      : null,
    sourceBaselinePreserved: true,
    harnessDelta: [...new Set([...changed, ...untracked])],
    fingerprints: [...new Set([...files, ...(expected ? changed : [])])].map(
      (file) => ({
        file,
        blob: git("hash-object", file),
        sha256: crypto
          .createHash("sha256")
          .update(fs.readFileSync(path.join(root, file)))
          .digest("hex"),
      }),
    ),
  };
}

function listeners() {
  return ["tcp", "tcp6"].flatMap((protocol) =>
    fs
      .readFileSync(`/proc/net/${protocol}`, "utf8")
      .trim()
      .split("\n")
      .slice(1)
      .map((line) => line.trim().split(/\s+/))
      .filter((columns) => columns[3] === "0A")
      .map((columns) => ({ protocol, address: columns[1] })),
  );
}

async function bounded(operation, timeout, name) {
  let timer;
  try {
    return await Promise.race([
      operation,
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`${name} timed out`)),
          timeout,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

function asset(url, origin, method, cache) {
  if (url.origin !== origin || method !== "GET") return false;
  const pathname = decodeURIComponent(url.pathname);
  const generated = `${cache.url}/deps/`;
  if (pathname.startsWith(generated)) {
    const basename = pathname.slice(generated.length);
    if (
      pathname !== url.pathname ||
      !/^[A-Za-z0-9_@.-]+\.js(?:\.map)?$/.test(basename)
    )
      return false;
    const filename = path.join(cache.directory, "deps", basename);
    let existing = filename;
    while (!fs.existsSync(existing)) existing = path.dirname(existing);
    return (
      fs.realpathSync(existing) === existing &&
      filename.startsWith(`${cache.directory}/deps/`)
    );
  }
  if (pathname === fixture || pathname === "/tests/e2e/compact-agentic.html")
    return true;
  if (
    pathname === "/@vite/client" ||
    pathname === "/@id/__vite-browser-external"
  )
    return true;
  if (!/^\/(src|node_modules|tests\/e2e)\//.test(pathname)) return false;
  if (
    pathname.startsWith("/tests/e2e/") &&
    ![
      fixture,
      "/tests/e2e/agentic-renderer-qualification.tsx",
      "/tests/e2e/compact-agentic.html",
      "/tests/e2e/compact-agentic.tsx",
    ].includes(pathname)
  )
    return false;
  const filename = path.resolve(gui, `.${pathname}`);
  if (!fs.existsSync(filename)) return false;
  return (
    fs.realpathSync(filename).startsWith(`${gui}/`) &&
    fs.statSync(filename).isFile()
  );
}

async function main() {
  const boundary = containment();
  assert.deepEqual(process.argv.slice(2, 3), ["--output"]);
  assert.ok([4, 8].includes(process.argv.length));
  let expected;
  if (process.argv.length === 8) {
    assert.equal(process.argv[4], "--expected-head");
    assert.equal(process.argv[6], "--expected-tree");
    assert.match(process.argv[5], /^[0-9a-f]{40}$/);
    assert.match(process.argv[7], /^[0-9a-f]{40}$/);
    expected = { head: process.argv[5], tree: process.argv[7] };
  }
  const output = path.resolve(root, process.argv[3]);
  const outputRoot = path.join(root, ".refact/ui-browser");
  assert.ok(output === outputRoot || output.startsWith(`${outputRoot}/`));
  fs.mkdirSync(output, { recursive: true });
  const attempt =
    new Date().toISOString().replace(/[:.]/g, "-") + "-" + crypto.randomUUID();
  const directory = path.join(output, attempt);
  fs.mkdirSync(directory);
  const cache = {
    directory: path.join(gui, "node_modules/.vite", attempt),
    url: `/node_modules/.vite/${attempt}`,
  };
  fs.mkdirSync(cache.directory, { recursive: true });
  assert.equal(fs.realpathSync(cache.directory), cache.directory);
  const names = [
    "setup",
    "network-canaries",
    "mount-smoke",
    "reports",
    "standalone",
    "question-v1",
    "question-legacy",
    "question-v2",
    "noncompact",
    ...[240, 360, 768, 1280].flatMap((width) =>
      ["light", "dark"].map((theme) => `geometry-${width}-${theme}`),
    ),
  ];
  const report = {
    attempt,
    boundary,
    limits: [
      "Network namespace and browser interception only; not a filesystem or AF_UNIX sandbox",
      "No golden baseline comparison",
      "Exact /@vite/client CSS-only adapter; production renderers/styles, not Vite HMR/client integration",
      "External admission and Rust gates NOT_RUN by this harness; overall qualification remains BLOCKED",
    ],
    externalGates: { admission: "NOT_RUN", rust: "NOT_RUN" },
    scenarios: Object.fromEntries(
      names.map((name) => [name, { status: "BLOCKED", checks: [] }]),
    ),
    intercepted: [],
    unexpectedDenials: [],
    serverDenials: [],
    pageErrors: [],
    consoleErrors: [],
    importErrors: [],
    screenshots: [],
    assets: [],
    cleanup: {},
    overallQualification: "BLOCKED",
  };
  let server, browser, context, page;
  let serverStart, browserStart, contextStart, listenerStart;
  let current = "setup";
  let halted = false;
  let cleanupPromise;
  const sockets = new Set();
  const commands = [];
  let allowCommand = false;
  const finishPath = path.join(directory, "result.json");
  const save = () =>
    fs.writeFileSync(finishPath, JSON.stringify(report, null, 2) + "\n");
  const cleanup = () =>
    (cleanupPromise ??= (async () => {
      const errors = [];
      for (const [name, close] of [
        [
          "context",
          async () => {
            await contextStart?.catch(() => undefined);
            await context?.close();
          },
        ],
        [
          "browser",
          async () => {
            await browserStart?.catch(() => undefined);
            await browser?.close();
          },
        ],
        [
          "vite",
          async () => {
            await serverStart?.catch(() => undefined);
            await listenerStart?.catch(() => undefined);
            for (const socket of sockets) socket.destroy();
            await server?.close();
            assert.ok(!server?.httpServer?.listening);
            assert.equal(sockets.size, 0);
            assert.deepEqual(listeners(), []);
          },
        ],
      ]) {
        try {
          await bounded(
            Promise.resolve().then(close),
            10000,
            `${name} cleanup`,
          );
          report.cleanup[name] = true;
        } catch (error) {
          report.cleanup[name] = false;
          errors.push(`${name}: ${error}`);
        }
      }
      assert.deepEqual(errors, []);
    })());
  const stop = () => {
    halted = true;
    report.scenarios[current].status = "FAIL";
    report.browserCriteria = "FAIL";
    report.error = "Attempt watchdog or termination signal";
    process.exitCode = 1;
    void cleanup()
      .catch((error) => {
        report.cleanup.error = String(error);
      })
      .finally(save);
  };
  const watchdog = setTimeout(stop, 270000);
  process.once("SIGTERM", stop);
  process.once("SIGINT", stop);
  try {
    report.source = source(expected);
    const local = (name) => path.join(gui, "node_modules", name);
    report.versions = Object.fromEntries(
      [
        "prettier",
        "vite",
        "react",
        "react-dom",
        "playwright",
        "@playwright/test",
      ].map((name) => [
        name,
        JSON.parse(
          fs.readFileSync(path.join(local(name), "package.json"), "utf8"),
        ).version,
      ]),
    );
    assert.equal(report.versions.prettier, "3.1.1");
    assert.equal(report.versions.vite, "5.3.2");
    assert.equal(report.versions.react, report.versions["react-dom"]);
    const { chromium } = require(local("playwright"));
    const expect = require(local("@playwright/test")).expect.configure({
      timeout: 10000,
    });
    assert.ok(
      fs.existsSync(chromium.executablePath()),
      "Existing Chromium binary required",
    );
    const { createServer } = await import(
      pathToFileURL(local("vite/dist/node/index.js")).href
    );
    const bootstrap = fs
      .readFileSync(path.join(gui, fixture), "utf8")
      .match(/<script data-offline-env>([\s\S]*?)<\/script>/g);
    assert.equal(bootstrap?.length, 1);
    const environmentPrelude = bootstrap[0]
      .replace(/^<script data-offline-env>/, "")
      .replace(/<\/script>$/, "");
    assert.deepEqual(listeners(), []);
    report.assetServer = {
      requestedBind: { port: 0, host: "127.0.0.1" },
      bindPath: "server.httpServer.listen",
      middlewareConfigured: false,
      pluginBuildStarted: false,
      listeningEvents: 0,
      ready: false,
      errors: [],
      proxy: {},
      hmr: false,
      ws: false,
      nodePreload: false,
    };
    serverStart = createServer({
      root: gui,
      configFile: false,
      envFile: false,
      publicDir: false,
      cacheDir: cache.directory,
      optimizeDeps: {
        entries: [
          "tests/e2e/agentic-renderer-qualification.html",
          "tests/e2e/compact-agentic.html",
        ],
      },
      logLevel: "error",
      esbuild: { jsx: "automatic" },
      resolve: { dedupe: ["react", "react-dom"] },
      define: {
        "process.env.NODE_ENV": JSON.stringify("development"),
        "process.env.DEBUG": JSON.stringify(""),
        __REFACT_LSP_PORT__: "0",
        __REFACT_CHAT_VERSION__: JSON.stringify({
          semver: "offline-fixture",
          commit: candidate,
        }),
      },
      server: {
        host: "127.0.0.1",
        port: 0,
        strictPort: true,
        proxy: {},
        hmr: false,
        ws: false,
        watch: null,
        fs: { strict: true, allow: [gui] },
      },
      plugins: [
        {
          name: "owned-assets-only",
          buildStart() {
            report.assetServer.pluginBuildStarted = true;
          },
          transformIndexHtml: {
            order: "pre",
            handler(_html, metadata) {
              if (metadata.path !== "/tests/e2e/compact-agentic.html") return;
              return [
                {
                  tag: "script",
                  attrs: { "data-offline-env": true },
                  children: environmentPrelude,
                  injectTo: "head-prepend",
                },
              ];
            },
          },
          configureServer(vite) {
            report.assetServer.middlewareConfigured = true;
            vite.middlewares.use((request, response, next) => {
              if (
                request.method === "GET" &&
                new URL(request.url, "http://fixture").pathname ===
                  "/@vite/client"
              ) {
                response.setHeader("Content-Type", "application/javascript");
                response.end(cssClient);
                return;
              }
              if (
                request.method !== "GET" ||
                /^\/v1(?:\/|$)/.test(request.url)
              ) {
                report.serverDenials.push({
                  method: request.method,
                  path: new URL(request.url, "http://fixture").pathname,
                });
                response.statusCode = 403;
                response.end(
                  "Offline asset server refuses API and non-GET requests",
                );
              } else next();
            });
          },
        },
      ],
    }).then((created) => {
      server = created;
      return created;
    });
    await serverStart;
    if (halted) {
      throw new Error("Stopped during Vite creation");
    }
    assert.ok(server.httpServer);
    assert.equal(server.httpServer.listening, false);
    server.httpServer.on("connection", (socket) => {
      sockets.add(socket);
      socket.once("close", () => sockets.delete(socket));
    });
    server.httpServer.on("error", (error) => {
      report.assetServer.errors.push({
        message: error.message,
        stack: error.stack,
      });
    });
    server.httpServer.on("listening", () => {
      report.assetServer.listeningEvents += 1;
    });
    await new Promise((resolve, reject) => {
      const complete = (error) => {
        clearTimeout(timer);
        server.httpServer.removeListener("error", failed);
        server.httpServer.removeListener("listening", ready);
        if (error) reject(error);
        else resolve();
      };
      const failed = (error) => complete(error);
      const ready = () => complete();
      const timer = setTimeout(
        () => complete(new Error("Owned HTTP listener timed out")),
        30000,
      );
      server.httpServer.once("error", failed);
      server.httpServer.once("listening", ready);
      listenerStart = Promise.resolve(server.httpServer.listen(0, "127.0.0.1"));
      listenerStart.catch(failed);
    });
    const address = server.httpServer.address();
    assert.ok(address && typeof address === "object");
    assert.equal(address.address, "127.0.0.1");
    assert.equal(address.family, "IPv4");
    assert.ok(Number.isInteger(address.port) && address.port > 0);
    assert.equal(server.httpServer.listening, true);
    assert.equal(report.assetServer.middlewareConfigured, true);
    assert.equal(report.assetServer.pluginBuildStarted, true);
    assert.equal(report.assetServer.listeningEvents, 1);
    assert.deepEqual(report.assetServer.errors, []);
    const ownedListeners = listeners();
    assert.deepEqual(ownedListeners, [
      {
        protocol: "tcp",
        address: `0100007F:${address.port
          .toString(16)
          .toUpperCase()
          .padStart(4, "0")}`,
      },
    ]);
    const origin = `http://127.0.0.1:${address.port}`;
    Object.assign(report.assetServer, {
      origin,
      address,
      listeners: ownedListeners,
      ready: true,
    });
    assert.equal(halted, false, "Attempt stopped");
    browserStart = chromium
      .launch({
        headless: true,
        timeout: 20000,
        env: { ...process.env },
        args: ["--no-proxy-server"],
      })
      .then((created) => {
        browser = created;
        return created;
      });
    await browserStart;
    if (halted) {
      throw new Error("Stopped during browser launch");
    }
    contextStart = browser
      .newContext({
        serviceWorkers: "block",
        reducedMotion: "reduce",
        viewport: { width: 1280, height: 900 },
      })
      .then((created) => {
        context = created;
        return created;
      });
    await contextStart;
    assert.equal(halted, false, "Attempt stopped");
    await context.grantPermissions(["clipboard-read", "clipboard-write"], {
      origin,
    });
    await context.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      const record = {
        method: request.method(),
        path: url.pathname,
        ownedOrigin: url.origin === origin,
      };
      if (
        url.origin === origin &&
        request.method() === "POST" &&
        url.pathname === "/v1/chats/question-v2/commands" &&
        allowCommand
      ) {
        const body = request.postDataJSON();
        assert.equal(request.headers().authorization, undefined);
        assert.equal(body.type, "submit_question_response");
        commands.push(body);
        report.intercepted.push({ ...record, action: "mock", body });
        return route.fulfill({
          status: commands.length === 1 ? 503 : 200,
          contentType: "application/json",
          body: commands.length === 1 ? "fixture send failure" : "{}",
        });
      }
      if (asset(url, origin, request.method(), cache)) {
        if (!report.assets.includes(url.pathname))
          report.assets.push(url.pathname);
        return route.continue();
      }
      const canary =
        current === "network-canaries" &&
        url.origin === origin &&
        url.pathname === "/v1/offline-negative-canary";
      report.intercepted.push({
        ...record,
        action: "deny",
        intentionalCanary: canary,
      });
      if (!canary) report.unexpectedDenials.push(record);
      return route.abort("blockedbyclient");
    });
    await context.routeWebSocket("**/*", (route) => {
      const url = new URL(route.url());
      const canary =
        current === "network-canaries" &&
        url.host === new URL(origin).host &&
        url.pathname === "/offline-negative-canary";
      const record = {
        method: "WS",
        path: url.pathname,
        intentionalCanary: canary,
      };
      report.intercepted.push(record);
      if (!canary) report.unexpectedDenials.push(record);
      route.close();
    });
    page = await context.newPage();
    page.setDefaultTimeout(10000);
    page.setDefaultNavigationTimeout(30000);
    page.on("pageerror", (error) =>
      report.pageErrors.push({
        scenario: current,
        message: error.message,
        stack: error.stack,
      }),
    );
    page.on("console", (message) => {
      if (message.type() !== "error") return;
      report.consoleErrors.push({
        scenario: current,
        message: message.text(),
        location: message.location(),
      });
    });
    page.on("requestfailed", (request) => {
      if (request.resourceType() !== "script") return;
      report.importErrors.push({
        scenario: current,
        url: request.url(),
        message: request.failure()?.errorText,
        stack: null,
      });
    });
    page.on("response", (response) => {
      if (
        response.request().resourceType() !== "script" ||
        response.status() < 400
      )
        return;
      report.importErrors.push({
        scenario: current,
        url: response.url(),
        message: `Module HTTP ${response.status()}`,
        stack: null,
      });
    });
    assert.equal(halted, false, "Attempt stopped");
    report.scenarios.setup = {
      status: "PASS",
      checks: [
        "Fresh verified namespace/all capabilities zero/NNP1",
        "Own strict ephemeral Vite listener and pre-page context HTTP/WS interception",
        "Existing local dependencies and Chromium",
      ],
    };
    const check = (text) => report.scenarios[current].checks.push(text);
    const scenario = async (name, operation) => {
      assert.equal(halted, false, "Attempt stopped");
      current = name;
      await operation();
      assert.equal(halted, false, "Attempt stopped");
      assert.deepEqual(report.unexpectedDenials, []);
      assert.deepEqual(report.pageErrors, []);
      assert.deepEqual(report.importErrors, []);
      assert.deepEqual(report.assetServer.errors, []);
      if (name === "mount-smoke")
        assert.deepEqual(
          report.consoleErrors.filter((error) => error.scenario === name),
          [],
        );
      report.scenarios[name].status = "PASS";
      save();
    };
    const shot = async (name) => {
      const filename = path.join(
        directory,
        `${report.screenshots.length}-${name}.png`,
      );
      try {
        await page.screenshot({
          path: filename,
          fullPage: true,
          timeout: 10000,
        });
        report.screenshots.push({ name, path: filename, status: "PASS" });
      } catch (error) {
        report.screenshots.push({
          name,
          path: filename,
          status: "FAIL",
          error: String(error),
        });
        throw error;
      }
    };
    const row = (id) => page.locator(`article[data-testid="${id}"]`);
    const trigger = (article) =>
      article.locator(
        'section[data-density="compact"] > div > button[aria-controls]',
      );
    const unique = async (locator) => {
      await expect(locator).toHaveCount(1);
      return locator;
    };
    const frames = () =>
      bounded(
        page.evaluate(
          () =>
            new Promise((resolve) =>
              requestAnimationFrame(() => requestAnimationFrame(resolve)),
            ),
        ),
        10000,
        "Renderer frames",
      );
    await scenario("network-canaries", async () => {
      await bounded(
        page.evaluate(async (origin) => {
          await fetch(`${origin}/v1/offline-negative-canary`).then(
            () => {
              throw new Error("HTTP canary escaped");
            },
            () => undefined,
          );
          await new Promise((resolve) => {
            const ws = new WebSocket(
              origin.replace("http:", "ws:") + "/offline-negative-canary",
            );
            ws.onclose = resolve;
          });
        }, origin),
        10000,
        "Browser network canaries",
      );
      for (const [method, pathname] of [
        ["GET", "/v1/offline-negative-canary"],
        ["POST", fixture],
      ]) {
        const status = await new Promise((resolve, reject) => {
          const request = http.request(
            origin + pathname,
            { method, agent: false },
            (response) => {
              response.resume();
              response.once("end", () => resolve(response.statusCode));
            },
          );
          request.setTimeout(10000, () =>
            request.destroy(new Error("Asset-server canary timed out")),
          );
          request.once("error", reject);
          request.end();
        });
        assert.equal(status, 403);
      }
      assert.equal(
        report.intercepted.filter((entry) => entry.intentionalCanary).length,
        2,
      );
      check(
        "Intentional HTTP/WS browser canaries denied; Vite GET /v1 and non-GET return 403",
      );
    });
    await scenario("mount-smoke", async () => {
      await page.goto(origin + fixture, { waitUntil: "load", timeout: 90000 });
      await page.waitForFunction(() =>
        ["ready", "FAIL"].includes(
          globalThis.__REFACT_RENDERER_IMPORT__?.status,
        ),
      );
      const imported = await page.evaluate(
        () => globalThis.__REFACT_RENDERER_IMPORT__,
      );
      report.fixtureImport = imported;
      if (imported?.status === "FAIL") report.importErrors.push(imported);
      assert.deepEqual(imported, { status: "ready" });
      const main = await unique(
        page.getByRole("main", {
          name: "Production renderer qualification",
          exact: true,
        }),
      );
      await expect(main).toBeVisible();
      await expect(main.locator("article[data-testid]")).toHaveCount(10);
      await expect(trigger(row("q2-unknown"))).toContainText(
        "Report unverified",
      );
      await expect(
        row("q2-standalone")
          .locator("p")
          .filter({ hasText: /^END OF REPORT$/ }),
      ).toBeVisible();
      await expect(
        row("question-v2-open").getByRole("button", {
          name: "Submit response",
          exact: true,
        }),
      ).toBeVisible();
      await frames();
      check(
        "Fixture module imported; exact main visible with ten production cases, rendered report tail and pending response controls; no page/console/import errors",
      );
      await shot("mount-smoke");
    });
    await scenario("reports", async () => {
      for (const [id, label, status] of [
        ["q2-unknown", "Report unverified", "idle"],
        ["q2-failure", "Reported failure", "error"],
        ["q2-success", "Reported success", "idle"],
        ["q2-malformed", "Report unverified", "idle"],
        ["q2-missing", "State unknown", "idle"],
      ]) {
        const article = await unique(row(id));
        const toggle = await unique(trigger(article));
        await expect(toggle).toHaveAttribute("aria-expanded", "false");
        await expect(toggle).toContainText(label);
        await expect(toggle).not.toContainText(
          /verified success|task completed/i,
        );
        await expect(
          article.locator('section[data-density="compact"]'),
        ).toHaveAttribute("data-status", status);
        await toggle.focus();
        await page.keyboard.press("Enter");
        await expect(toggle).toHaveAttribute("aria-expanded", "true");
        await expect(
          article.getByText("No verification evidence", { exact: true }),
        ).toBeVisible();
        const more = await unique(
          article.getByRole("button", { name: "More details", exact: true }),
        );
        await expect(more).toHaveAttribute("aria-expanded", "false");
        await more.click();
        await expect(more).toHaveAttribute("aria-expanded", "true");
        if (["q2-unknown", "q2-failure", "q2-success"].includes(id)) {
          const tail = await unique(
            article.locator("p").filter({ hasText: /^END OF REPORT$/ }),
          );
          await expect(tail).toBeVisible();
          await expect(
            article.getByRole("link", {
              name: "Source reference",
              exact: true,
            }),
          ).toHaveAttribute("href", "#report-source");
          const copy = await unique(
            article.getByRole("button", { name: "Copy report", exact: true }),
          );
          await copy.click();
          await expect
            .poll(() => page.evaluate(() => navigator.clipboard.readText()))
            .toBe(markdown);
          await expect(
            article.getByText("renderer.ts", { exact: true }),
          ).toBeVisible();
        }
        const technical = await unique(
          article.getByRole("button", {
            name: "Technical details",
            exact: true,
          }),
        );
        await technical.click();
        await expect(technical).toHaveAttribute("aria-expanded", "true");
        const raw = page
          .locator(`[id="${await technical.getAttribute("aria-controls")}"]`)
          .locator("code");
        const payload = JSON.stringify({
          type: "task_done",
          summary: "Renderer delivery reported",
          report: markdown,
          files_changed: ["src/renderer.ts"],
          ...(id === "q2-success"
            ? { success: true }
            : id === "q2-failure"
              ? { success: false }
              : {}),
        });
        await expect(raw).toHaveText(
          id === "q2-missing"
            ? ["{}"]
            : [id === "q2-malformed" ? "{broken" : payload, "{}"],
        );
        for (const code of await raw.all()) await expect(code).toBeVisible();
        check(
          `${id}: truthful ${label}/${status}, closed→expanded→More details→exact raw content`,
        );
        await toggle.click();
        await expect(toggle).toHaveAttribute("aria-expanded", "false");
      }
      await shot("reports");
    });
    await scenario("standalone", async () => {
      const article = await unique(row("q2-standalone"));
      const toggle = await unique(
        article.locator("section > div > button[aria-controls]"),
      );
      await expect(toggle).toHaveAttribute("aria-expanded", "true");
      await expect(toggle).toContainText("Reported failure");
      await expect(article.locator('[data-density="compact"]')).toHaveCount(0);
      await expect(
        article.getByRole("button", { name: "More details", exact: true }),
      ).toHaveCount(0);
      const content = await unique(
        article.locator('[class*="contentUnbounded"]'),
      );
      assert.deepEqual(
        await content.evaluate((element) => ({
          maxHeight: getComputedStyle(element).maxHeight,
          overflowY: getComputedStyle(element).overflowY,
        })),
        { maxHeight: "none", overflowY: "visible" },
      );
      await expect(
        article.locator("p").filter({ hasText: /^END OF REPORT$/ }),
      ).toBeVisible();
      await shot("standalone");
      check(
        "Explicit standalone TaskDone remains expanded, failure-labeled and unbounded through report tail",
      );
    });
    for (const id of ["question-v1", "question-legacy"])
      await scenario(id, async () => {
        const article = await unique(row(id));
        const toggle = await unique(trigger(article));
        await expect(toggle).toHaveAttribute("aria-expanded", "false");
        await toggle.focus();
        await page.keyboard.press("Enter");
        await expect(toggle).toHaveAttribute("aria-expanded", "true");
        const choice = await unique(
          article.getByRole("radio", { name: "Compact choice", exact: true }),
        );
        await choice.focus();
        await page.keyboard.press("Space");
        await expect(choice).toBeChecked();
        if (id === "question-legacy")
          await article
            .getByPlaceholder("Add any extra context...", { exact: true })
            .fill("Keep this draft");
        await toggle.focus();
        await page.keyboard.press("Space");
        await expect(toggle).toHaveAttribute("aria-expanded", "false");
        await expect(
          article.getByRole("button", { name: "Submit Answers", exact: true }),
        ).toBeHidden();
        await page.keyboard.press("Enter");
        await expect(toggle).toHaveAttribute("aria-expanded", "true");
        await expect(choice).toBeChecked();
        if (id === "question-legacy")
          await expect(
            article.getByPlaceholder("Add any extra context...", {
              exact: true,
            }),
          ).toHaveValue("Keep this draft");
        assert.equal(commands.length, 0);
        await expect(
          article.getByRole("button", { name: "More details", exact: true }),
        ).toHaveCount(0);
        check(
          "Keyboard Enter/Space disclosure; choice and optional text survive close/open; no command without Submit",
        );
      });
    await scenario("question-v2", async () => {
      const article = await unique(row("question-v2"));
      const toggle = await unique(trigger(article));
      await expect(toggle).toHaveAttribute("aria-expanded", "false");
      await toggle.focus();
      await page.keyboard.press("Enter");
      await expect(toggle).toHaveAttribute("aria-expanded", "true");
      const no = await unique(
        article.getByRole("radio", { name: "No", exact: true }),
      );
      await no.focus();
      await page.keyboard.press("Space");
      const choice = await unique(
        article.getByRole("radio", { name: "Compact choice", exact: true }),
      );
      await choice.focus();
      await page.keyboard.press("Space");
      const comments = await unique(
        article.getByRole("textbox", { name: /Overall comments/ }),
      );
      await comments.fill("Keep this context exactly.");
      await toggle.click();
      await expect(toggle).toHaveAttribute("aria-expanded", "false");
      await toggle.focus();
      await page.keyboard.press("Enter");
      await expect(toggle).toHaveAttribute("aria-expanded", "true");
      await expect(no).toBeChecked();
      await expect(choice).toBeChecked();
      await expect(comments).toHaveValue("Keep this context exactly.");
      assert.equal(commands.length, 0);
      allowCommand = true;
      const submit = await unique(
        article.getByRole("button", { name: "Submit response", exact: true }),
      );
      await submit.click();
      await expect(article.getByRole("alert")).toContainText("503");
      assert.equal(commands.length, 1);
      const expected = {
        type: "submit_question_response",
        tool_call_id: "question-v2",
        asked_at_ms: 42,
        response: {
          answers: [
            {
              id: "continue",
              disposition: "answered",
              answer: false,
              explanation: "",
            },
            {
              id: "scope",
              disposition: "answered",
              answer: "Compact choice",
              explanation: "",
            },
          ],
          overall_comments: "Keep this context exactly.",
        },
        client_request_id: commands[0].client_request_id,
      };
      assert.match(expected.client_request_id, /^[0-9a-f-]{36}$/);
      assert.deepEqual(commands[0], expected);
      await toggle.click();
      await expect(toggle).toHaveAttribute("aria-expanded", "false");
      await toggle.click();
      await expect(toggle).toHaveAttribute("aria-expanded", "true");
      await frames();
      assert.equal(
        commands.length,
        1,
        "No automatic retry after mocked failure and disclosure changes",
      );
      await submit.click();
      await expect(
        article.getByText("Response sent, awaiting confirmation.", {
          exact: true,
        }),
      ).toBeVisible();
      assert.deepEqual(commands, [expected, expected]);
      await expect(submit).toBeDisabled();
      await expect(no).toBeDisabled();
      await toggle.click();
      await expect(toggle).toHaveAttribute("aria-expanded", "false");
      await toggle.click();
      await expect(toggle).toHaveAttribute("aria-expanded", "true");
      await frames();
      assert.equal(
        commands.length,
        2,
        "No automatic resend while awaiting confirmation",
      );
      allowCommand = false;
      await shot("questions");
      check(
        "No command before Submit; actual exact body preserves false/Compact choice/comments; explicit retry retains client_request_id; no automatic resend across state changes",
      );
    });
    await scenario("noncompact", async () => {
      const article = await unique(row("question-v2-open"));
      await expect(
        article.locator('section[data-density="default"]'),
      ).toHaveAttribute("data-open", "true");
      await expect(article.locator("button[aria-controls]")).toHaveCount(0);
      await expect(
        article.getByRole("radio", { name: "No", exact: true }),
      ).toBeVisible();
      await expect(
        article.getByRole("button", { name: "Submit response", exact: true }),
      ).toBeVisible();
      check("Noncompact pending response controls visible without disclosure");
    });
    for (const width of [240, 360, 768, 1280])
      for (const theme of ["light", "dark"])
        await scenario(`geometry-${width}-${theme}`, async () => {
          await page.setViewportSize({ width, height: 900 });
          await page.goto(
            `${origin}/tests/e2e/compact-agentic.html?theme=${theme}`,
            { waitUntil: "load" },
          );
          await expect(
            page.locator(`[data-host="web"][data-appearance="${theme}"]`),
          ).toBeVisible();
          const toggle = await unique(trigger(row("compact-task-done")));
          await expect(toggle).toHaveAttribute("aria-expanded", "false");
          const geometry = await page.getByRole("main").evaluate((main) => ({
            pageOverflow: document.documentElement.scrollWidth > innerWidth + 1,
            overflowingRows: Array.from(
              main.querySelectorAll("article[data-testid]"),
            )
              .filter((element) => {
                const rect = element.getBoundingClientRect();
                return (
                  rect.left < -1 ||
                  rect.right > innerWidth + 1 ||
                  element.scrollWidth > element.clientWidth + 1
                );
              })
              .map((element) => element.getAttribute("data-testid")),
          }));
          assert.deepEqual(geometry, {
            pageOverflow: false,
            overflowingRows: [],
          });
          const height = await row("compact-task-done")
            .locator('section[data-density="compact"]')
            .evaluate((element) => element.getBoundingClientRect().height);
          assert.ok(Math.abs(height - 32) <= 1);
          await shot(`geometry-${width}-${theme}`);
          check(
            `Existing representative production fixture: ${width}px ${theme}, no page/row horizontal overflow, collapsed 32px report; screenshot captured`,
          );
        });
    assert.deepEqual(report.unexpectedDenials, []);
    assert.deepEqual(report.pageErrors, []);
    assert.deepEqual(report.importErrors, []);
    assert.equal(report.serverDenials.length, 2);
    assert.equal(halted, false, "Attempt stopped");
    report.browserCriteria = "PASS";
  } catch (error) {
    halted = true;
    report.scenarios[current].status = "FAIL";
    report.error = String(error.stack);
    report.browserCriteria = "FAIL";
    if (page && !page.isClosed()) {
      try {
        const imported = await bounded(
          page.evaluate(() => globalThis.__REFACT_RENDERER_IMPORT__),
          5000,
          "Fixture import diagnostics",
        );
        if (imported) report.fixtureImport = imported;
        if (imported?.status === "FAIL") report.importErrors.push(imported);
      } catch (failure) {
        report.importDiagnosticError = String(failure.stack);
      }
      const filename = path.join(directory, `failure-${current}.png`);
      try {
        await page.screenshot({
          path: filename,
          fullPage: true,
          timeout: 5000,
        });
        report.screenshots.push({
          name: current,
          path: filename,
          status: "PASS",
          diagnosticOnly: true,
        });
      } catch (failure) {
        report.screenshots.push({
          name: current,
          path: filename,
          status: "FAIL",
          error: String(failure),
        });
      }
    }
    process.exitCode = 1;
  } finally {
    clearTimeout(watchdog);
    process.removeListener("SIGTERM", stop);
    process.removeListener("SIGINT", stop);
    try {
      await cleanup();
    } catch (error) {
      report.cleanup.error = String(error);
      report.browserCriteria = "FAIL";
      process.exitCode = 1;
    }
    save();
    process.stdout.write(
      JSON.stringify({
        result: finishPath,
        browserCriteria: report.browserCriteria,
        overallQualification: report.overallQualification,
        externalGates: report.externalGates,
        scenarios: report.scenarios,
        error: report.error,
        cleanup: report.cleanup,
      }) + "\n",
    );
  }
}

main().catch((error) => {
  process.stderr.write(`Offline renderer refused: ${error.stack}\n`);
  process.exitCode = 1;
});
