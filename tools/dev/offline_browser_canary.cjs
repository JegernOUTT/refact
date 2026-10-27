const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");

async function main() {
  assert.equal(
    fs.readlinkSync("/proc/self/ns/net"),
    process.env.REFACT_OFFLINE_NETNS,
  );
  assert.notEqual(
    process.env.REFACT_OFFLINE_NETNS,
    process.env.REFACT_OFFLINE_HOST_NETNS,
  );
  const { chromium } = require(
    path.resolve(__dirname, "../../refact-agent/gui/node_modules/playwright"),
  );
  const counters = { connections: 0, bytes: 0, requests: 0, upgrades: 0 };
  const listener = http.createServer((_request, response) => {
    counters.requests += 1;
    response.end("owned browser canary");
  });
  listener.on("connection", (socket) => {
    counters.connections += 1;
    socket.on("data", (data) => {
      counters.bytes += data.length;
    });
  });
  listener.on("upgrade", (_request, socket) => {
    counters.upgrades += 1;
    socket.destroy();
  });
  await new Promise((resolve) => listener.listen(0, "127.0.0.1", resolve));
  const port = listener.address().port;
  await new Promise((resolve, reject) =>
    http
      .get(`http://127.0.0.1:${port}/ready`, { agent: false }, (response) => {
        response.resume();
        response.once("end", resolve);
      })
      .once("error", reject),
  );
  assert.equal(counters.requests, 1);
  const readiness = { ...counters };
  const browser = await chromium.launch({ headless: true });
  const intercepted = [];
  const context = await browser.newContext({ serviceWorkers: "block" });
  await context.route("**/*", (route) => {
    intercepted.push(route.request().url());
    return route.abort("blockedbyclient");
  });
  await context.routeWebSocket("**/*", (route) => {
    intercepted.push(route.url());
    route.close();
  });
  const page = await context.newPage();
  try {
    const results = await page.evaluate(async (port) => {
      const url = `http://127.0.0.1:${port}/`;
      const checks = [];
      await import(
        `data:text/javascript,${encodeURIComponent(
          `await fetch(${JSON.stringify(url + "module")}).catch(()=>{})`,
        )}`
      );
      checks.push("module evaluation fetch settled");
      await fetch(url + "fetch").catch(() => checks.push("fetch rejected"));
      await new Promise((resolve) => {
        const xhr = new XMLHttpRequest();
        xhr.open("GET", url + "xhr");
        xhr.onloadend = () => {
          checks.push("XHR settled");
          resolve();
        };
        xhr.send();
      });
      await new Promise((resolve) => {
        const stream = new EventSource(url + "events");
        stream.onerror = () => {
          stream.close();
          checks.push("EventSource closed on error");
          resolve();
        };
      });
      await new Promise((resolve) => {
        const socket = new WebSocket(`ws://127.0.0.1:${port}/socket`);
        socket.onclose = () => {
          checks.push("WebSocket closed");
          resolve();
        };
      });
      await new Promise((resolve) => {
        const source = `fetch(${JSON.stringify(
          url + "worker",
        )}).catch(()=>{}).then(()=>postMessage("done"))`;
        const objectUrl = URL.createObjectURL(
          new Blob([source], { type: "text/javascript" }),
        );
        const worker = new Worker(objectUrl);
        worker.onmessage = () => {
          worker.terminate();
          URL.revokeObjectURL(objectUrl);
          checks.push("Worker fetch settled and terminated");
          resolve();
        };
      });
      return checks;
    }, port);
    await context.close();
    await browser.close();
    await new Promise((resolve) => listener.close(resolve));
    const unexpected = Object.fromEntries(
      Object.keys(counters).map((key) => [key, counters[key] - readiness[key]]),
    );
    assert.deepEqual(unexpected, {
      connections: 0,
      bytes: 0,
      requests: 0,
      upgrades: 0,
    });
    for (const suffix of [
      "module",
      "fetch",
      "xhr",
      "events",
      "socket",
      "worker",
    ]) {
      assert.ok(
        intercepted.some((url) => url.endsWith("/" + suffix)),
        `${suffix} interception required`,
      );
    }
    const report = {
      passed: true,
      readiness,
      unexpected,
      checks: results,
      intercepted,
      namespace: process.env.REFACT_OFFLINE_NETNS,
    };
    fs.writeFileSync(process.argv[2], JSON.stringify(report, null, 2));
    process.stdout.write(JSON.stringify(report, null, 2) + "\n");
  } finally {
    await context.close();
    await browser.close();
    if (listener.listening)
      await new Promise((resolve) => listener.close(resolve));
  }
}

main().catch((error) => {
  process.stderr.write(String(error.stack) + "\n");
  process.exitCode = 1;
});
