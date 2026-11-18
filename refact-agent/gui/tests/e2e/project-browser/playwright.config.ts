import { defineConfig } from "@playwright/test";
import { assertIsolated } from "./fixture";

assertIsolated();

export default defineConfig({
  testDir: "..",
  testMatch: "project-browser.spec.ts",
  workers: 1,
  retries: 0,
  timeout: 240_000,
  reporter: "list",
  use: { actionTimeout: 20_000, navigationTimeout: 25_000 },
  outputDir: "../../../test-results/project-browser",
});
