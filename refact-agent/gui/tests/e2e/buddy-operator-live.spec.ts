import {
  expect,
  test,
  type Locator,
  type Page,
  type Request,
} from "@playwright/test";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";

const questionPath = "/v1/buddy/decisions/questions/b02-native/b02-question";
const effect = "/scratch/project/.refact/commands/b02-browser.md";
const origin = "http://127.0.0.1:4173";
const headers = { Authorization: "Bearer scratch-only-not-a-secret" };

async function openDesk(page: Page) {
  await page.goto("/tests/e2e/buddy-operator-live.html");
  await page.getByRole("button", { name: /Browser scratch change/ }).click();
  await expect(
    page.getByRole("heading", { name: "Browser scratch report" }),
  ).toBeVisible();
}

async function chooseYes(decision: Locator) {
  const answer = decision.getByRole("radiogroup", {
    name: "Answer to question 1",
    exact: true,
  });
  await answer.getByText("Yes", { exact: true }).click();
  await expect(
    answer.getByRole("radio", { name: "Yes", exact: true }),
  ).toBeChecked();
}

// There is deliberately no route interception/fulfillment here. Browser → Vite
// proxy → production router all live in one default-deny kernel network namespace.
test("actual GUI effect, receipt, two native clients and restart with intelligence off", async ({
  page,
  browser,
}) => {
  test.setTimeout(120_000);
  const responses: { url: string; status: number; body: string }[] = [];
  page.on("response", async (response) => {
    if (new URL(response.url()).pathname.startsWith("/v1/buddy/")) {
      responses.push({
        url: response.url(),
        status: response.status(),
        body: await response.text().catch(() => "response body unavailable"),
      });
    }
  });
  const requests: string[] = [];
  const external: string[] = [];
  const observe = (client: Page) =>
    client.on("request", (request) => {
      const url = new URL(request.url());
      if (url.origin !== origin) external.push(url.href);
      if (url.pathname.startsWith("/v1/"))
        requests.push(`${request.method()} ${url.pathname}`);
    });
  observe(page);
  const secondContext = await browser.newContext({
    baseURL: origin,
    extraHTTPHeaders: headers,
    serviceWorkers: "block",
  });
  const second = await secondContext.newPage();
  const secondTime = new Date("2025-01-01T00:00:00Z");
  await second.clock.install({ time: secondTime });
  await second.clock.setFixedTime(secondTime);
  await second.clock.pauseAt(secondTime);
  await second.clock.setSystemTime(secondTime);
  const secondGets = new Set<Request>();
  second.on("request", (request) => {
    if (
      request.method() === "GET" &&
      new URL(request.url()).pathname.startsWith("/v1/buddy/")
    )
      secondGets.add(request);
  });
  second.on("requestfinished", (request) => secondGets.delete(request));
  second.on("requestfailed", (request) => secondGets.delete(request));
  let secondElapsed = 0;
  const settleSecond = async (control: Locator) => {
    await expect
      .poll(
        async () => {
          secondElapsed += 16;
          expect(secondElapsed).toBeLessThan(3000);
          await second.clock.runFor(16);
          return (
            secondGets.size === 0 &&
            (await control.count()) === 1 &&
            (await control.isVisible()) &&
            (await control.isEnabled())
          );
        },
        { intervals: [50] },
      )
      .toBe(true);
  };
  observe(second);
  try {
    await openDesk(page);
    await second.goto("/tests/e2e/buddy-operator-live.html");
    const secondReport = second.getByRole("button", {
      name: /Browser scratch change/,
    });
    await settleSecond(secondReport);
    await secondReport.click();
    await settleSecond(
      second.getByRole("heading", { name: "Browser scratch report" }),
    );
    const decision = (client: Page) =>
      client.getByRole("listitem", { name: "Question", exact: true });
    for (const client of [page, second]) {
      await expect(decision(client)).toContainText("Continue scratch work?");
      if (client === second)
        await settleSecond(
          decision(second).getByRole("radio", { name: "Yes", exact: true }),
        );
      await chooseYes(decision(client));
      await expect(
        decision(client).getByRole("button", {
          name: "Submit response",
          exact: true,
        }),
      ).toBeEnabled();
    }
    expect(existsSync(effect)).toBe(false);
    await page.getByRole("button", { name: "Preview exact change" }).click();
    const confirm = page.getByRole("checkbox", {
      name: "I confirm this exact scope, version, and change.",
    });
    await expect(confirm).toBeVisible();
    expect(existsSync(effect)).toBe(false);
    await expect(
      page.getByRole("button", { name: "Grant and execute once" }),
    ).toBeDisabled();
    await confirm.check();
    const rights = page.getByRole("checkbox", {
      name: /I authorize exactly these native/,
    });
    if (await rights.count()) await rights.check();
    const sent = page.waitForRequest(
      (request) =>
        request.method() === "POST" &&
        new URL(request.url()).pathname === "/v1/buddy/actions/execute",
    );
    const finished = page.waitForResponse(
      (response) =>
        response.request().method() === "POST" &&
        new URL(response.url()).pathname === "/v1/buddy/actions/execute",
    );
    await page.getByRole("button", { name: "Grant and execute once" }).click();
    const execute: unknown = (await sent).postDataJSON();
    const response = await finished;
    expect(response.status()).toBe(200);
    const receipt = (await response.json()).data.Finished.execution;
    // Native markdown is durably written but intentionally not activated/reloaded.
    // Partial is the exact DiskDoneReloadNeeded contract, not an arbitrary result.
    expect(receipt.status).toBe("partial");
    expect(receipt.effects).toHaveLength(1);
    expect(receipt.effects[0]).toMatchObject({
      resource: {
        resource_kind: "native_markdown",
        resource_id: "commands/b02-browser",
      },
      state: "applied",
    });
    expect(receipt.execution_id).toBe(
      (execute as { execution_id: string }).execution_id,
    );
    await expect(
      page.getByText("Observed result: Partial", { exact: true }),
    ).toBeVisible();
    const bytes = readFileSync(effect, "utf8");
    expect(bytes).toContain("Browser scratch effect");
    const digest = createHash("sha256").update(bytes).digest("hex");
    expect(receipt.effects[0].resource.version).toBe(digest);
    await expect(
      page.getByText(`Proposed version: ${digest}`, { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText(
        "Disk changes may require a separate reload. This control does not activate or reload them.",
      ),
    ).toBeVisible();
    writeFileSync(
      "/scratch/browser-evidence.json",
      JSON.stringify({ execute, receipt, digest, bytes }),
    );

    const restart = async (sequence: number) => {
      writeFileSync(
        `/scratch/restart-${sequence}`,
        "restart actual scratch worker",
      );
      await expect
        .poll(() => existsSync(`/scratch/restarted-${sequence}`))
        .toBe(true);
      expect(readFileSync(effect, "utf8")).toBe(bytes);
    };
    await restart(1);
    // This same browser context retains production execution tracking, not a fixture receipt.
    await openDesk(page);
    await expect(
      page.getByRole("button", { name: "Preview exact change" }),
    ).toBeDisabled();
    await page.getByRole("button", { name: "Refresh saved execution" }).click();
    await expect(
      page.getByText("Observed result: Partial", { exact: true }),
    ).toBeVisible();
    await expect(decision(page)).toContainText("Continue scratch work?");
    await chooseYes(decision(page));
    const pendingRefresh = second.waitForResponse(
      (response) =>
        response.request().method() === "GET" &&
        new URL(response.url()).pathname === questionPath,
    );
    await decision(second)
      .getByRole("button", { name: "Refresh decision status" })
      .click();
    const pendingResponse = await pendingRefresh;
    expect((await pendingResponse.json()).question.status).toBe("pending");
    await pendingResponse.finished();
    await settleSecond(
      decision(second).getByRole("button", {
        name: "Submit response",
        exact: true,
      }),
    );
    await expect(
      decision(second).getByRole("button", {
        name: "Submit response",
        exact: true,
      }),
    ).toBeEnabled();
    const accepted = page.waitForResponse(
      (response) =>
        response.request().method() === "POST" &&
        new URL(response.url()).pathname === questionPath,
    );
    await decision(page)
      .getByRole("button", { name: "Submit response", exact: true })
      .click();
    expect((await accepted).status()).toBe(202);
    await expect(
      decision(page).getByText("Answered — confirmed by the chat."),
    ).toBeVisible();
    const stale = second.waitForResponse(
      (response) =>
        response.request().method() === "POST" &&
        new URL(response.url()).pathname === questionPath,
    );
    await decision(second)
      .getByRole("button", { name: "Submit response", exact: true })
      .click();
    expect((await stale).status()).toBe(409);
    await second.clock.resume();
    await expect(decision(second).getByRole("alert")).toContainText(
      "The chat hasn't confirmed this answer",
    );
    await decision(second)
      .getByRole("button", { name: "Refresh decision status" })
      .click();
    await expect(
      decision(second).getByText("Answered — confirmed by the chat."),
    ).toBeVisible();
    await expect(
      decision(second).getByRole("button", {
        name: "Submit response",
        exact: true,
      }),
    ).toBeDisabled();
    const durableUrl = `${questionPath}?project_root=/scratch/project`;
    const durable = await secondContext.request.get(durableUrl);
    expect(durable.ok()).toBe(true);
    const resolved: unknown = await durable.json();
    expect(resolved).toMatchObject({ question: { status: "answered" } });
    await restart(2);
    const restored = await secondContext.request.get(durableUrl);
    expect(restored.ok()).toBe(true);
    expect(await restored.json()).toEqual(resolved);
    const staleAfterRestart = await secondContext.request.post(questionPath, {
      data: {
        project_root: "/scratch/project",
        asked_at_ms: 7,
        client_request_id: "browser-stale-after-restart",
        answers: [{ id: "continue", answer: true }],
      },
    });
    expect(staleAfterRestart.status()).toBe(409);
    await decision(second)
      .getByRole("button", { name: "Refresh decision status" })
      .click();
    await expect(
      decision(second).getByText("Answered — confirmed by the chat."),
    ).toBeVisible();
    await openDesk(page);
    await expect(
      page.getByRole("button", { name: "Preview exact change" }),
    ).toBeDisabled();
    await page.getByRole("button", { name: "Refresh saved execution" }).click();
    await expect(
      page.getByText("Observed result: Partial", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("region", { name: "Questions", exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Refresh reports", exact: true }),
    ).toBeEnabled();
    expect(
      requests.filter(
        (request) => request === "POST /v1/buddy/actions/execute",
      ),
    ).toHaveLength(1);
    expect(
      requests.filter((request) => request === `POST ${questionPath}`),
    ).toHaveLength(2);
    expect(external).toEqual([]);
    writeFileSync(
      "/scratch/browser-requests.json",
      JSON.stringify({ requests, external, resolved }),
    );
  } finally {
    writeFileSync(
      "/scratch/browser-diagnostics.json",
      JSON.stringify(
        {
          responses,
          requests,
          external,
          dom: await page
            .locator("body")
            .innerText()
            .catch(() => "page closed; see retained trace"),
        },
        null,
        2,
      ),
    );
    await secondContext.close();
  }
});
