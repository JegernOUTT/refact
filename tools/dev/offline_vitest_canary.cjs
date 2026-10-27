const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");
const { spawn } = require("node:child_process");

async function main() {
  assert.equal(
    fs.readlinkSync("/proc/self/ns/net"),
    process.env.REFACT_OFFLINE_NETNS,
  );
  const gui = path.resolve(__dirname, "../../refact-agent/gui");
  const directory = path.join(gui, ".refact/offline-qualification");
  fs.mkdirSync(directory, { recursive: true });
  const result = path.join(directory, "vitest-result.json");
  if (fs.existsSync(result)) fs.unlinkSync(result);
  const counters = { connections: 0, requests: 0, bytes: 0, upgrades: 0 };
  const listener = http.createServer((_request, response) => {
    counters.requests += 1;
    response.end("owned fixture");
  });
  listener.on("connection", (socket) => {
    counters.connections += 1;
    socket.on("data", (bytes) => {
      counters.bytes += bytes.length;
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
  fs.writeFileSync(
    path.join(directory, "transport.test.ts"),
    `import { expect, it } from "vitest";
import { readlinkSync, writeFileSync } from "node:fs";
import { connect } from "node:net";
const state = globalThis as typeof globalThis & { __REFACT_OFFLINE_NETWORK__?: boolean; __REFACT_OFFLINE_VIOLATIONS__?: number };
if (!state.__REFACT_OFFLINE_NETWORK__) throw new Error("Missing pre-import guard");
if (readlinkSync("/proc/self/ns/net") !== process.env.REFACT_OFFLINE_NETNS) throw new Error("Wrong worker namespace");
let moduleBlocked = false;
try { await fetch("http://127.0.0.1:${port}/module-evaluation"); } catch { moduleBlocked = true; }
if (!moduleBlocked || !(state.__REFACT_OFFLINE_VIOLATIONS__! > 0)) throw new Error("Module evaluation was not blocked");
it("requires transport denial before module evaluation and in the actual configured fork", () => {
  const before = state.__REFACT_OFFLINE_VIOLATIONS__!;
  expect(() => connect({host:"127.0.0.1",port:${port}})).toThrow("Offline verification blocked");
  expect(state.__REFACT_OFFLINE_VIOLATIONS__).toBe(before + 1);
  writeFileSync(${JSON.stringify(
    result,
  )}, JSON.stringify({namespace:process.env.REFACT_OFFLINE_NETNS,moduleBlocked,denials:state.__REFACT_OFFLINE_VIOLATIONS__,execArgv:process.execArgv}));
});
`,
  );
  const child = spawn(
    process.execPath,
    [
      path.join(gui, "node_modules/vitest/vitest.mjs"),
      "run",
      ".refact/offline-qualification/transport.test.ts",
    ],
    {
      cwd: gui,
      env: {
        ...process.env,
        NODE_OPTIONS: `--require=${path.join(
          __dirname,
          "offline_network.cjs",
        )}`,
      },
      stdio: "inherit",
    },
  );
  const watchdog = setTimeout(() => child.kill(), 60000);
  try {
    const code = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", resolve);
    });
    assert.equal(code, 0);
    await new Promise((resolve) => listener.close(resolve));
    const unexpected = Object.fromEntries(
      Object.keys(counters).map((key) => [key, counters[key] - readiness[key]]),
    );
    assert.deepEqual(unexpected, {
      connections: 0,
      requests: 0,
      bytes: 0,
      upgrades: 0,
    });
    const report = {
      passed: true,
      readiness,
      unexpected,
      worker: JSON.parse(fs.readFileSync(result, "utf8")),
    };
    fs.writeFileSync(process.argv[2], JSON.stringify(report, null, 2));
    process.stdout.write(JSON.stringify(report, null, 2) + "\n");
  } finally {
    clearTimeout(watchdog);
    if (child.exitCode === null) {
      child.kill();
      await new Promise((resolve) => child.once("exit", resolve));
    }
    if (listener.listening)
      await new Promise((resolve) => listener.close(resolve));
    fs.rmSync(path.join(directory, "transport.test.ts"), { force: true });
  }
}

main().catch((error) => {
  process.stderr.write(String(error.stack) + "\n");
  process.exitCode = 1;
});
