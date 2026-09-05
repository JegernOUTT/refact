import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: "reconstructed-history.spec.ts",
  workers: 1,
  use: {
    ...devices["Desktop Chrome"],
    baseURL: "http://127.0.0.1:4187",
    viewport: { width: 1280, height: 1000 },
    trace: "on",
  },
  webServer: {
    command:
      "REFACT_LSP_URL=http://127.0.0.1:1 npm run dev -- --host 127.0.0.1 --port 4187 --strictPort",
    url: "http://127.0.0.1:4187/tests/e2e/route-showcase.html",
    reuseExistingServer: false,
  },
});
