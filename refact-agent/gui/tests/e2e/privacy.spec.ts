import { expect, test } from "@playwright/test";

type ShowcaseRequest = {
  path: string;
  method: string;
  body: unknown;
};

const privacySettingsPath =
  "/tests/e2e/route-showcase.html?route=settings&settings=privacy";

async function showcaseRequests(page: import("@playwright/test").Page) {
  return page.evaluate<ShowcaseRequest[]>(() =>
    (
      window as unknown as {
        __routeShowcaseRequests: ShowcaseRequest[];
      }
    ).__routeShowcaseRequests.slice(),
  );
}

test.describe("privacy surfaces", () => {
  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  test("renders the zone grid and saves a destination toggle", async ({
    page,
  }) => {
    await page.goto(privacySettingsPath);

    await page.getByRole("button", { name: "Show matrix" }).click();

    const grid = page.getByRole("table", {
      name: "Zone destination permissions",
    });
    await expect(grid).toBeVisible();
    await expect(grid.getByRole("columnheader")).toHaveCount(3);
    await expect(
      grid.getByRole("columnheader", { name: /Trusted provider/ }),
    ).toBeVisible();
    await expect(
      grid.getByRole("columnheader", { name: /Build MCP/ }),
    ).toBeVisible();

    await page.getByRole("button", { name: /MCP servers/ }).click();
    await page.getByRole("button", { name: /Build MCP/ }).click();
    await page.getByLabel("Send secrets to Build MCP").click();

    await expect
      .poll(async () => {
        const requests = await showcaseRequests(page);
        return requests.filter(
          (request) =>
            request.path === "/v1/privacy/policy" && request.method === "POST",
        ).length;
      })
      .toBe(1);

    const saveRequest = (await showcaseRequests(page)).find(
      (request) =>
        request.path === "/v1/privacy/policy" && request.method === "POST",
    );
    expect(saveRequest?.body).toEqual({
      blocked: ["*.blocked"],
      zones: [
        {
          name: "secrets",
          patterns: [".env*"],
          send_to: ["trusted", "build-mcp"],
          on_shell_read: "withhold",
        },
        {
          name: "normal",
          patterns: ["**"],
          send_to: ["*"],
          on_shell_read: "ask",
        },
      ],
      subagents: { report_declassifies: true },
      tool_access: { providers: {} },
    });
  });

  test("shows why runtime observation is unavailable", async ({ page }) => {
    await page.goto(privacySettingsPath);

    await expect(page.getByText("Degraded attribution")).toBeVisible();
    await expect(page.getByText("PTRACE_TRACEME is unavailable")).toBeVisible();
    await expect(page.getByText("Runtime active")).not.toBeVisible();
  });

  test("reveals withheld output without firing a request", async ({ page }) => {
    await page.goto("/tests/e2e/route-showcase.html?route=privacy-withheld");
    await expect(
      page.getByText("Ran, exit 0 — output withheld, it read .env."),
    ).toBeVisible();
    await expect(page.getByText("TOKEN=local-secret")).not.toBeVisible();
    const revealRequests: string[] = [];
    page.on("request", (request) => revealRequests.push(request.url()));

    await page.getByRole("button", { name: "Show me" }).click();

    await expect(page.getByText("TOKEN=local-secret")).toBeVisible();
    expect(revealRequests).toEqual([]);
  });

  test("offers exactly the two block recovery actions", async ({ page }) => {
    await page.goto("/tests/e2e/route-showcase.html?route=privacy-block");

    await expect(
      page.getByText("This model cannot receive this step"),
    ).toBeVisible();
    await expect(page.getByRole("button")).toHaveCount(2);
    await expect(
      page.getByRole("button", { name: "Switch to an allowed model" }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", {
        name: "Branch clean chat from before step 3",
      }),
    ).toBeVisible();

    await page
      .getByRole("button", { name: "Switch to an allowed model" })
      .click();
    await page
      .getByRole("button", {
        name: "Branch clean chat from before step 3",
      })
      .click();

    await expect
      .poll(() =>
        page.evaluate<string[]>(() =>
          (
            window as unknown as {
              __routeShowcasePrivacyActions: string[];
            }
          ).__routeShowcasePrivacyActions.slice(),
        ),
      )
      .toEqual(["switch-model", "branch-clean-chat"]);
  });
});
