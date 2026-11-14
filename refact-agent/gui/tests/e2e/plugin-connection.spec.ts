import { expect, test } from "@playwright/test";

const PROVIDER_KEY = "fixture-provider-secret-not-for-daemon";
const DAEMON_ORIGIN = "http://127.0.0.1:19488";
const DAEMON_PATHS = ["status", "workers", "settings", "update/status"].map(
  (path) => `/daemon/v1/${path}`,
);

test.use({ serviceWorkers: "block" });

const startupCases = [
  {
    issue: "engine_start_failed",
    title: "Refact engine could not start",
  },
  {
    issue: "workspace_required",
    title: "Open a folder to use Refact",
  },
  {
    issue: "workspace_busy",
    title: "Another Refact engine is using this folder",
  },
  { issue: "connecting", title: "Connecting to Refact" },
  { issue: "installing", title: "Refact" },
] as const;

for (const host of ["vscode", "jetbrains"]) {
  for (const width of [240, 360, 1280]) {
    for (const theme of ["light", "dark"]) {
      for (const startup of startupCases) {
        test(`${host} ${startup.issue} opens diagnostics at ${width}px in ${theme}`, async ({
          page,
          baseURL,
        }) => {
          const mutations: string[] = [];
          const workerRequests: string[] = [];
          const daemonRequests: string[] = [];
          const unexpectedRequests: string[] = [];
          const credentialLeaks: string[] = [];
          const credentialInspections: Promise<void>[] = [];
          const transient =
            startup.issue === "connecting" || startup.issue === "installing";
          await page.setViewportSize({ width, height: transient ? 500 : 900 });
          await page.route("**/*", async (route) => {
            const request = route.request();
            const url = new URL(request.url());
            const isWorker = /^\/(?:p\/[^/]+\/)?v1(?:\/|$)/.test(url.pathname);
            const isDaemon = url.pathname.startsWith("/daemon/v1/");
            const identity = `${request.method()} ${url.origin}${url.pathname}`;
            if (!["GET", "HEAD", "OPTIONS"].includes(request.method())) {
              mutations.push(identity);
            }
            if (isWorker) workerRequests.push(identity);
            credentialInspections.push(
              request.allHeaders().then((headers) => {
                if (
                  [
                    request.url(),
                    JSON.stringify(headers),
                    request.postData() ?? "",
                  ].some((value) => value.includes(PROVIDER_KEY))
                )
                  credentialLeaks.push(identity);
              }),
            );
            if (url.origin !== DAEMON_ORIGIN || !isDaemon) {
              if (url.origin === baseURL && !isWorker && !isDaemon) {
                await route.continue();
              } else {
                unexpectedRequests.push(identity);
                await route.abort();
              }
              return;
            }
            if (
              !DAEMON_PATHS.includes(url.pathname) ||
              request.method() !== "GET"
            ) {
              unexpectedRequests.push(identity);
              await route.abort();
              return;
            }
            daemonRequests.push(url.pathname);
            const headers = { "access-control-allow-origin": "*" };
            if (url.pathname === "/daemon/v1/status") {
              await route.fulfill({
                headers,
                json: {
                  pid: 42,
                  port: 19488,
                  version: "diagnostics-fixture",
                  started_at_ms: 1794400000000,
                  uptime_secs: 60,
                  workers: 0,
                  cron_pending: {},
                },
              });
            } else if (url.pathname === "/daemon/v1/workers") {
              await route.fulfill({ headers, json: [] });
            } else {
              await route.fulfill({
                headers,
                status: 401,
                json: { error: "Unauthorized" },
              });
            }
          });
          await page.goto(
            `/tests/e2e/plugin-connection.html?host=${host}&issue=${startup.issue}&theme=${theme}`,
          );
          await expect(
            page.getByRole("heading", { name: startup.title, exact: true }),
          ).toBeVisible();
          await expect(
            page.getByText("Backend server unreachable", { exact: true }),
          ).toHaveCount(0);
          if (!transient) {
            await expect(
              page.getByRole("button", { name: "Continue", exact: true }),
            ).toBeDisabled();
          }
          await page
            .getByRole("button", { name: "Component versions", exact: true })
            .click();
          const versionRow = (label: string) =>
            page
              .locator("dt")
              .filter({ hasText: new RegExp(`^${label}$`) })
              .locator("..");
          await expect(versionRow("IDE plugin")).toContainText(
            "8.7.0-preview.2",
          );
          await expect(versionRow("Plugin UI")).toContainText(
            "Loaded UI bundle",
          );
          await expect(versionRow("Requested runtime")).toContainText(
            "Policy, not an observed version",
          );
          await expect(versionRow("Selected executable")).toContainText(
            "8.6.4",
          );
          await expect(versionRow("Running daemon")).toContainText("8.6.3");
          await expect(versionRow("Running daemon")).toContainText(
            "Last observed",
          );
          for (const label of [
            "Installed runtime",
            "Staged runtime",
            "Project engine",
          ]) {
            await expect(versionRow(label)).toContainText("Not detected");
          }
          await versionRow("Project engine").scrollIntoViewIfNeeded();
          await expect(versionRow("Project engine")).toBeInViewport();
          expect(
            await page.evaluate(
              () => document.documentElement.scrollWidth <= innerWidth,
            ),
          ).toBe(true);
          if (startup.issue === "workspace_busy") {
            await expect(
              page.getByText("PID 4242 · HTTP port 8005", { exact: true }),
            ).toBeVisible();
          }
          await page
            .getByRole("button", {
              name: "Open daemon diagnostics",
              exact: true,
            })
            .click();
          await expect(
            page.getByRole("heading", { name: "Refact Daemon", exact: true }),
          ).toBeVisible();
          const daemonStatus = page
            .getByRole("heading", { name: "Daemon status", exact: true })
            .locator("xpath=ancestor::*[dl][1]");
          await expect(
            daemonStatus.getByText("diagnostics-fixture", { exact: true }),
          ).toBeVisible();
          await page
            .getByRole("button", { name: "Component versions", exact: true })
            .click();
          await expect(versionRow("Running daemon")).toContainText(
            "diagnostics-fixture",
          );
          await expect(versionRow("Running daemon")).not.toContainText("8.6.3");
          await expect(versionRow("Installed runtime")).toContainText(
            "Not detected",
          );
          await expect(
            page.getByText("http://127.0.0.1:19488", { exact: true }).first(),
          ).toBeVisible();
          await expect
            .poll(() =>
              ["status", "workers", "settings", "update/status"].every((path) =>
                daemonRequests.includes(`/daemon/v1/${path}`),
              ),
            )
            .toBe(true);
          await expect
            .poll(() =>
              page.evaluate(
                () => document.documentElement.scrollWidth <= innerWidth,
              ),
            )
            .toBe(true);
          await page.getByRole("button", { name: "Back", exact: true }).click();
          await expect(
            page.getByRole("heading", { name: startup.title, exact: true }),
          ).toBeVisible();
          if (!transient) {
            await expect(
              page.getByRole("button", { name: "Continue", exact: true }),
            ).toBeDisabled();
          }
          await page.waitForLoadState("networkidle");
          await Promise.all(credentialInspections);
          expect(mutations).toEqual([]);
          expect(workerRequests).toEqual([]);
          expect(unexpectedRequests).toEqual([]);
          expect(credentialLeaks).toEqual([]);
        });
      }
    }
  }
}
