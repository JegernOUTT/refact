const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const net = require("node:net");
const { fork } = require("node:child_process");
const { Worker, isMainThread, parentPort } = require("node:worker_threads");
const { pathToFileURL } = require("node:url");

const guard = path.join(__dirname, "offline_network.cjs");
const gui = path.resolve(__dirname, "../../refact-agent/gui");

async function attempts(port) {
  assert.equal(globalThis.__REFACT_OFFLINE_NETWORK__, true);
  assert.equal(
    fs.readlinkSync("/proc/self/ns/net"),
    process.env.REFACT_OFFLINE_NETNS,
  );
  const url = `http://127.0.0.1:${port}/canary`;
  const results = [];
  const blocked = async (name, operation) => {
    const before = globalThis.__REFACT_OFFLINE_VIOLATIONS__;
    try {
      await operation();
      assert.fail(`${name} unexpectedly succeeded`);
    } catch (error) {
      assert.match(
        String(error.stack) + String(error.cause?.stack),
        /REFACT_OFFLINE_NETWORK|Offline verification blocked/,
      );
      assert.ok(
        globalThis.__REFACT_OFFLINE_VIOLATIONS__ > before,
        `${name} must reach the transport guard`,
      );
      results.push(name);
    }
  };
  await blocked("net object", () => net.connect({ port, host: "127.0.0.1" }));
  await blocked("net numeric", () =>
    new net.Socket().connect(port, "127.0.0.1"),
  );
  await blocked("net normalized", () =>
    new net.Socket().connect([{ port, host: "127.0.0.1" }]),
  );
  await blocked("ESM net", async () =>
    (await import("node:net")).connect({ port, host: "127.0.0.1" }),
  );
  await blocked("TLS", () =>
    require("node:tls").connect({ port, host: "127.0.0.1" }),
  );
  await blocked("HTTP", () => require("node:http").get(url));
  await blocked("HTTPS", () =>
    require("node:https").get(url.replace("http:", "https:")),
  );
  await blocked("Node fetch", () => fetch(url));
  await blocked(
    "module evaluation fetch",
    () =>
      import(
        `data:text/javascript,${encodeURIComponent(
          `await fetch(${JSON.stringify(url)})`,
        )}`
      ),
  );
  await blocked("DNS", () =>
    require("node:dns").lookup("canary.invalid", () => {}),
  );
  const udp = require("node:dgram").createSocket("udp4");
  await blocked("UDP", () => udp.send("canary", port, "127.0.0.1"));
  udp.close();
  const { Window } = await import(
    pathToFileURL(path.join(gui, "node_modules/happy-dom/lib/index.js")).href
  );
  const window = new Window({ url });
  await blocked("DOM fetch", () => window.fetch(url));
  await blocked(
    "DOM XHR",
    () =>
      new Promise((resolve, reject) => {
        const xhr = new window.XMLHttpRequest();
        xhr.open("GET", url);
        xhr.onload = resolve;
        xhr.onerror = () =>
          reject(new Error("Offline verification blocked DOM XHR"));
        xhr.send();
      }),
  );
  await window.happyDOM.close();
  const wsPath = require.resolve("ws", {
    paths: [path.join(gui, "node_modules")],
  });
  await blocked(
    "WebSocket",
    () =>
      new Promise((resolve, reject) => {
        const socket = new (require(wsPath))(`ws://127.0.0.1:${port}/canary`);
        socket.once("open", resolve);
        socket.once("error", reject);
      }),
  );
  const msw = require(
    require.resolve("msw", { paths: [path.join(gui, "node_modules")] }),
  );
  const mswNode = require(
    require.resolve("msw/node", { paths: [path.join(gui, "node_modules")] }),
  );
  const server = mswNode.setupServer(
    msw.http.get(url, () => msw.HttpResponse.json({ mocked: true })),
  );
  server.listen({ onUnhandledRequest: "error" });
  try {
    assert.deepEqual(await (await fetch(url)).json(), { mocked: true });
    results.push("MSW fulfilled without transport");
    await assert.rejects(fetch(`${url}/unhandled`));
    results.push("MSW unmatched rejected");
    server.resetHandlers(
      msw.http.get(`${url}/replacement`, () =>
        msw.HttpResponse.json({ mocked: true }),
      ),
    );
    await assert.rejects(fetch(url));
    results.push("MSW reset rejected");
  } finally {
    server.close();
  }
  await blocked("MSW closed", () => fetch(url));
  const before = globalThis.__REFACT_OFFLINE_VIOLATIONS__;
  try {
    net.connect({ port, host: "127.0.0.1" });
  } catch {}
  assert.equal(globalThis.__REFACT_OFFLINE_VIOLATIONS__, before + 1);
  results.push("caught transport violation remains counted");
  return {
    checks: results,
    denied_transports: globalThis.__REFACT_OFFLINE_VIOLATIONS__,
  };
}

function childResult(child) {
  return new Promise((resolve, reject) => {
    let report;
    child.on("message", (message) => {
      report = message;
    });
    child.once("error", reject);
    child.once("exit", (code) =>
      code === 0 && report
        ? resolve(report)
        : reject(new Error(`Canary child exit ${code}`)),
    );
  });
}

async function main() {
  const mode = process.argv[2];
  if (!isMainThread || mode === "child") {
    const result = await attempts(Number(process.env.REFACT_CANARY_PORT));
    if (parentPort) parentPort.postMessage(result);
    else process.send(result);
    return;
  }
  assert.equal(
    fs.readlinkSync("/proc/self/ns/net"),
    process.env.REFACT_OFFLINE_NETNS,
  );
  assert.notEqual(
    process.env.REFACT_OFFLINE_NETNS,
    process.env.REFACT_OFFLINE_HOST_NETNS,
  );
  const counters = { connections: 0, bytes: 0, requests: 0, upgrades: 0 };
  const listener = require("node:http").createServer((_request, response) => {
    counters.requests += 1;
    response.end("owned canary listener");
  });
  listener.on("upgrade", (_request, socket) => {
    counters.upgrades += 1;
    socket.destroy();
  });
  listener.on("connection", (socket) => {
    counters.connections += 1;
    socket.on("data", (data) => {
      counters.bytes += data.length;
    });
  });
  await new Promise((resolve) => listener.listen(0, "127.0.0.1", resolve));
  const port = listener.address().port;
  await new Promise((resolve, reject) => {
    require("node:http")
      .get(`http://127.0.0.1:${port}/ready`, { agent: false }, (response) => {
        response.resume();
        response.once("end", resolve);
      })
      .once("error", reject);
  });
  assert.equal(counters.connections, 1);
  assert.equal(counters.requests, 1);
  const readiness = { ...counters };
  const env = {
    ...process.env,
    REFACT_CANARY_PORT: String(port),
    NODE_OPTIONS: `--require=${guard}`,
  };
  const children = [];
  let timedOut = false;
  const watchdog = setTimeout(() => {
    timedOut = true;
    for (const child of children) {
      if (child instanceof Worker) void child.terminate();
      else child.kill();
    }
    listener.close();
  }, 20000);
  try {
    const child = fork(__filename, ["child"], {
      env,
      stdio: ["ignore", "inherit", "inherit", "ipc"],
    });
    children.push(child);
    const forked = await childResult(child);
    const worker = new Worker(__filename, {
      env,
      execArgv: ["--require", guard],
    });
    children.push(worker);
    const threaded = await childResult(worker);
    assert.equal(timedOut, false);
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
    const report = {
      passed: true,
      listener_readiness: readiness,
      unexpected,
      namespace: process.env.REFACT_OFFLINE_NETNS,
      host_namespace: process.env.REFACT_OFFLINE_HOST_NETNS,
      forked,
      threaded,
      limits: [
        "Kernel network namespace is the primary containment",
        "Preload guards Node transports before module imports",
        "Filesystem and inherited credentials are not sandboxed",
        "Browser EventSource and context interception require a separate canary",
      ],
    };
    const output = process.argv[3];
    if (output) fs.writeFileSync(output, JSON.stringify(report, null, 2));
    process.stdout.write(JSON.stringify(report, null, 2) + "\n");
  } finally {
    clearTimeout(watchdog);
    for (const child of children) {
      if (child instanceof Worker) await child.terminate();
      else if (child.exitCode === null) {
        child.kill();
        await new Promise((resolve) => child.once("exit", resolve));
      }
    }
    if (listener.listening)
      await new Promise((resolve) => listener.close(resolve));
  }
}

main().catch((error) => {
  process.stderr.write(String(error.stack) + "\n");
  process.exitCode = 1;
});
