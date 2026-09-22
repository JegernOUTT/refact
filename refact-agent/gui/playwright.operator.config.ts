import { defineConfig } from "@playwright/test";
import { readFileSync, readlinkSync, existsSync } from "node:fs";

// Fail closed: this config must never start a browser against a host service.
if (
  process.env.B02_BROWSER !== "1" ||
  process.env.HOME !== "/scratch/home" ||
  !process.env.B02_HOST_NET ||
  readlinkSync("/proc/self/ns/net") === process.env.B02_HOST_NET ||
  existsSync("/home") ||
  existsSync("/run") ||
  readFileSync("/proc/net/dev", "utf8")
    .split("\n")
    .filter((line) => line.includes(":"))
    .some((line) => line.split(":")[0].trim() !== "lo")
) {
  throw new Error("B02 REFUSED: browser requires the whole-worker namespace");
}

export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: "buddy-operator-live.spec.ts",
  workers: 1,
  retries: 0,
  reporter: "list",
  outputDir: "/scratch/results",
  use: {
    baseURL: "http://127.0.0.1:4173",
    extraHTTPHeaders: { Authorization: "Bearer scratch-only-not-a-secret" },
    serviceWorkers: "block",
    channel: "chromium",
    trace: "retain-on-failure",
    launchOptions: { chromiumSandbox: true },
  },
  webServer: {
    command:
      'node --input-type=module -e \'import {createServer} from "vite"; const server = await createServer({cacheDir:"/scratch/cache/vite",server:{host:"127.0.0.1",port:4173,strictPort:true}}); await server.listen();\'',
    url: "http://127.0.0.1:4173/tests/e2e/buddy-operator-live.html",
    reuseExistingServer: false,
  },
});
