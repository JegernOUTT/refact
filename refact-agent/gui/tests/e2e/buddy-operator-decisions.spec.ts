import { expect, test } from "@playwright/test";
import {
  buildApiUrl,
  getEngineEndpointIdentity,
  hasReadyPluginBackend,
} from "../../src/services/refact/apiUrl";

for (const host of ["web", "vscode", "jetbrains"] as const) {
  test(`${host} shared HTTP decision URL retains project proxy and native identity`, () => {
    const config = {
      host,
      lspUrl: "http://127.0.0.1:8123/project/scratch",
      backendReady: true,
      connectionStatus: "ready" as const,
    };
    const path = `/v1/buddy/decisions/questions/${encodeURIComponent(
      "chat/a",
    )}/${encodeURIComponent("tool:b")}`;
    expect(buildApiUrl(config, path)).toBe(
      "http://127.0.0.1:8123/project/scratch/v1/buddy/decisions/questions/chat%2Fa/tool%3Ab",
    );
    expect(getEngineEndpointIdentity(config)).not.toBe(
      getEngineEndpointIdentity({
        ...config,
        lspUrl: "http://127.0.0.1:8123/project/other",
      }),
    );
    if (host !== "web") {
      expect(hasReadyPluginBackend({ ...config, backendReady: false })).toBe(
        false,
      );
    }
  });
}

test("web decision acknowledgment is not resolution; native takeover and reconnect never replay answer", async ({
  page,
  baseURL,
}) => {
  const origin = new URL(baseURL ?? "http://127.0.0.1:4173").origin;
  let status = "pending";
  let posts = 0;
  let unavailable = false;
  const denied: string[] = [];
  const question = () => ({
    project_root: "/scratch/operator",
    chat_id: "chat-a",
    question: {
      tool_call_id: "tool-a",
      asked_at_ms: 123,
      status,
      questions: [
        { id: "proceed", type: "yes_no", text: "Proceed with scratch check?" },
      ],
    },
  });
  await page.route("**/*", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.origin !== origin) {
      denied.push(url.href);
      return route.abort();
    }
    if (!url.pathname.startsWith("/v1/")) return route.continue();
    if (url.pathname === "/v1/buddy/decisions/current") {
      return route.fulfill({
        json: {
          schema_version: 1,
          project_root: "/scratch/operator",
          coverage: "loaded_native_sessions",
          truncated: false,
          questions: [question()],
        },
      });
    }
    if (url.pathname === "/v1/buddy/decisions/questions/chat-a/tool-a") {
      if (request.method() === "POST") {
        posts += 1;
        expect(request.postDataJSON()).toEqual({
          project_root: "/scratch/operator",
          asked_at_ms: 123,
          client_request_id: expect.any(String),
          answers: [{ id: "proceed", answer: true }],
        });
        return route.fulfill({ status: 202, json: { queued: true } });
      }
      expect(url.searchParams.get("project_root")).toBe("/scratch/operator");
      if (unavailable)
        return route.fulfill({
          status: 503,
          json: { error: "fixture disconnected" },
        });
      return route.fulfill({ json: question() });
    }
    return route.fulfill({
      status: 503,
      json: { error: "unconfigured fixture route" },
    });
  });
  await page.goto("/tests/e2e/buddy-operator-preview.html?connected=1");
  const decision = page.getByRole("article", { name: "Native decision" });
  await decision.getByRole("radio", { name: "Yes", exact: true }).check();
  await decision
    .getByRole("button", { name: "Submit answer", exact: true })
    .click();
  await expect(
    decision.getByText("Answer queued.", { exact: false }),
  ).toBeVisible();
  await expect(
    decision.getByText("Answered — confirmed by the native chat."),
  ).toHaveCount(0);
  expect(posts).toBe(1);
  unavailable = true;
  await decision
    .getByRole("button", { name: "Refresh decision status" })
    .click();
  await expect(decision.getByRole("alert")).toBeVisible();
  await expect(
    decision.getByRole("button", { name: "Submit answer", exact: true }),
  ).toBeDisabled();
  status = "cancelled";
  unavailable = false;
  await decision
    .getByRole("button", { name: "Refresh decision status" })
    .click();
  await expect(
    decision.getByText(
      "Cancelled in the native chat. No answer will be applied.",
    ),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByText("Cancelled in the native chat. No answer will be applied."),
  ).toBeVisible();
  expect(posts).toBe(1);
  expect(denied).toEqual([]);
});
