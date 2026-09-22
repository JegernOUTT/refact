import { defineConfig, devices } from "@playwright/test";

const port = process.env.REFACT_E2E_PORT ?? "4173";
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: "./tests/e2e",
  testIgnore: "buddy-operator-live.spec.ts",
  fullyParallel: true,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: `npm run test:e2e:serve -- --port ${port} --strictPort`,
    url: `${baseURL}/tests/e2e/route-showcase.html`,
    reuseExistingServer: !process.env.CI && !process.env.REFACT_E2E_PORT,
    timeout: 120_000,
  },
});
