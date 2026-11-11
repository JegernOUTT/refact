import { expect, test, type Page } from "@playwright/test";
const fixture = "/tests/e2e/content-routing.html";

async function expectRouteReady(page: Page, hash: RegExp) {
  await expect(page).toHaveURL(hash);
  await expect(page.getByTestId("pending")).toHaveText("false");
  await expect(page.getByTestId("error")).toBeEmpty();
  await page.waitForLoadState("load");
}
test("Back and Forward restore content after state-driven navigation", async ({
  page,
}) => {
  await page.goto(`${fixture}#/settings/privacy`);
  await expect(page.getByTestId("page")).toHaveText("privacy settings");
  await expectRouteReady(page, /#\/settings\/privacy$/);
  await page.getByRole("button", { name: "Open file from workspace" }).click();
  await expect(page.getByTestId("surface")).toHaveText(
    "file:/workspace/example.ts",
  );
  await expectRouteReady(page, /#\/file\/%2Fworkspace%2Fexample.ts$/);
  await expect(page).toHaveURL(/#\/file\/%2Fworkspace%2Fexample.ts$/);
  await page.goBack({ waitUntil: "commit" });
  await expect(page.getByTestId("page")).toHaveText("privacy settings");
  await expectRouteReady(page, /#\/settings\/privacy$/);
  await page.goForward({ waitUntil: "commit" });
  await expect(page.getByTestId("page")).toHaveText("chat");
  await expect(page.getByTestId("surface")).toHaveText(
    "file:/workspace/example.ts",
  );
  await expectRouteReady(page, /#\/file\/%2Fworkspace%2Fexample.ts$/);
});

test("direct open, reload, Back and Forward restore focused content", async ({
  page,
}) => {
  await page.goto(`${fixture}#/settings/privacy`);
  await expect(page.getByTestId("page")).toHaveText("privacy settings");
  await expectRouteReady(page, /#\/settings\/privacy$/);
  await page.getByRole("link", { name: "File", exact: true }).click();
  await expect(page.getByTestId("surface")).toHaveText(
    "file:/workspace/example.ts",
  );
  await expectRouteReady(page, /#\/file\/%2Fworkspace%2Fexample.ts$/);
  await page.reload();
  await expect(page.getByTestId("surface")).toHaveText(
    "file:/workspace/example.ts",
  );
  await expectRouteReady(page, /#\/file\/%2Fworkspace%2Fexample.ts$/);
  await page.goBack({ waitUntil: "commit" });
  await expect(page.getByTestId("page")).toHaveText("privacy settings");
  await expectRouteReady(page, /#\/settings\/privacy$/);
  await page.goForward({ waitUntil: "commit" });
  await expect(page.getByTestId("surface")).toHaveText(
    "file:/workspace/example.ts",
  );
  await expectRouteReady(page, /#\/file\/%2Fworkspace%2Fexample.ts$/);
});
test("copyable legacy hrefs omit secrets and middle-click creates an independent tab", async ({
  page,
  context,
}) => {
  await page.goto(`${fixture}?token=private#/settings/privacy`);
  const link = page.getByRole("link", { name: "File", exact: true });
  await expect(link).toHaveAttribute(
    "href",
    `${fixture}#/file/%2Fworkspace%2Fexample.ts`,
  );
  await expect(
    page.getByRole("link", { name: "Chat", exact: true }),
  ).toHaveAttribute("href", `${fixture}#/chat/linked`);
  const newTab = context.waitForEvent("page");
  await link.click({ button: "middle" });
  const child = await newTab;
  await expect(child.getByTestId("surface")).toHaveText(
    "file:/workspace/example.ts",
  );
  expect(child.url()).not.toContain("private");
  await expect(page.getByTestId("page")).toHaveText("privacy settings");
  await expectRouteReady(page, /#\/settings\/privacy$/);
});

test("task-associated chat identity survives reload and subsequent planner selection", async ({
  page,
}) => {
  await page.route("**/v1/trajectories/*?metadata_only=true", (route) => {
    const id = new URL(route.request().url()).pathname.split("/").at(-1);
    return route.fulfill({
      json: {
        id,
        title: "Planner",
        mode: "task_planner",
        task_meta: { task_id: "task-1", role: "planner" },
      },
    });
  });
  await page.route("**/v1/tasks/task-1", (route) =>
    route.fulfill({ json: { meta: { id: "task-1", name: "Task" } } }),
  );
  await page.route("**/v1/tasks/task-1/board", (route) =>
    route.fulfill({
      json: {
        columns: [],
        cards: [
          {
            id: "T-1",
            title: "Card",
            column: "todo",
            priority: "P1",
            depends_on: [],
            instructions: "Work",
            assignee: null,
            agent_chat_id: null,
            status_updates: [],
            final_report: null,
            created_at: "",
            started_at: null,
            completed_at: null,
            target_files: [],
          },
        ],
      },
    }),
  );
  await page.route("**/v1/tasks/task-1/trajectories/planner", (route) =>
    route.fulfill({ json: [{ id: "planner-1" }, { id: "planner-2" }] }),
  );
  await page.goto(`${fixture}#/chat/planner-1`);
  await expect(page.getByTestId("task-chat")).toHaveText("planner-1");
  await expect(page).toHaveURL(/#\/chat\/planner-1$/);
  await page.reload();
  await expect(page.getByTestId("task-chat")).toHaveText("planner-1");
  await page.getByRole("button", { name: "Select second planner" }).click();
  await expect(page).toHaveURL(/#\/chat\/planner-2$/);
  await page.getByRole("link", { name: "Task", exact: true }).click();
  await expect(page.getByTestId("task-view")).toHaveText("board");
  await expect(page).toHaveURL(/#\/task\/task-1$/);
  await page.getByRole("link", { name: "Card", exact: true }).click();
  await expect(page.getByTestId("task-card")).toHaveText("T-1");
  await expect(page).toHaveURL(/#\/task\/task-1\/card\/T-1$/);
});
