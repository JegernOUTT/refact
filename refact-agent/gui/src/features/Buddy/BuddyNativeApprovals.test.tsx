import { http, HttpResponse } from "msw";
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BuddyNativeApprovals } from "./BuddyNativeApprovals";

const session = {
  chat_id: "chat-a",
  source_version: "a".repeat(64),
  browser_request_id: null,
  warnings: [],
  tools: [
    {
      type: "confirmation",
      tool_call_id: "tool-1",
      tool_name: "shell",
      command: "cat private",
      rule: "Privacy approval required: command read guarded files",
    },
  ],
};
const snapshot = {
  schema_version: 1,
  project_root: "/scratch",
  engine_identity: "engine-a",
  coverage: "loaded_native_sessions",
  truncated: false,
  warnings: [],
  sessions: [session],
};
const config = {
  config: { host: "vscode" as const, lspPort: 8001, themeProps: {} },
};

describe("Buddy native approvals without intelligence", () => {
  it("requires an explicit selection and never treats queue acceptance as resolution", async () => {
    const commands: unknown[] = [];
    server.use(
      http.get("*/v1/buddy/approvals/current", () =>
        HttpResponse.json(snapshot),
      ),
      http.post("*/v1/buddy/approvals/chat-a/response", async ({ request }) => {
        commands.push(await request.json());
        return HttpResponse.json({ status: "queued" }, { status: 202 });
      }),
      http.get("*/v1/buddy/status", () =>
        HttpResponse.json({ error: "mode down" }, { status: 503 }),
      ),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    const choice = await screen.findByRole("radio", {
      name: "Accept this request",
    });
    await waitFor(() => expect(choice).toBeEnabled());
    const submit = screen.getByRole("button", { name: "Submit tool decision" });
    expect(submit).toBeDisabled();
    expect(choice).not.toBeChecked();
    expect(screen.getByText(/command already ran/)).toBeTruthy();
    fireEvent.click(choice);
    fireEvent.click(submit);
    fireEvent.click(submit);
    expect(
      await screen.findByText(/Response queued, not resolved/),
    ).toBeTruthy();
    expect(commands).toHaveLength(1);
    expect(commands[0]).toMatchObject({
      project_root: "/scratch",
      engine_identity: "engine-a",
      source_version: "a".repeat(64),
      response: { kind: "tool", tool_call_id: "tool-1", accepted: true },
    });
    expect(submit).toBeDisabled();
  });

  it("has no default browser consent and forwards only explicitly selected categories", async () => {
    const commands: unknown[] = [];
    server.use(
      http.get("*/v1/buddy/approvals/current", () =>
        HttpResponse.json({
          ...snapshot,
          sessions: [
            { ...session, tools: [], browser_request_id: "browser-1" },
          ],
        }),
      ),
      http.post("*/v1/buddy/approvals/chat-a/response", async ({ request }) => {
        commands.push(await request.json());
        return HttpResponse.json({ status: "queued" }, { status: 202 });
      }),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    const checkbox = await screen.findByRole("checkbox", {
      name: "Include screenshot",
    });
    await waitFor(() => expect(checkbox).toBeEnabled());
    for (const input of screen.getAllByRole("checkbox"))
      expect(input).not.toBeChecked();
    const send = screen.getByRole("button", {
      name: "Send with selected context",
    });
    expect(send).toBeDisabled();
    fireEvent.click(checkbox);
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: /I confirm this context selection/,
      }),
    );
    fireEvent.click(send);
    await screen.findByText(/Response queued, not resolved/);
    expect(commands[0]).toMatchObject({
      response: {
        kind: "browser",
        pending_message_id: "browser-1",
        include_actions: false,
        include_console: false,
        include_network: false,
        include_mutations: false,
        include_screenshot: true,
      },
    });
  });

  it("clears selections after root, engine or source changes and disables controls on failed refresh", async () => {
    let current = snapshot;
    let offline = false;
    server.use(
      http.get("*/v1/buddy/approvals/current", () =>
        offline
          ? HttpResponse.json({ error: "offline" }, { status: 503 })
          : HttpResponse.json(current),
      ),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    await screen.findByRole("radio", { name: "Accept this request" });
    for (const change of [
      { project_root: "/other" },
      { engine_identity: "engine-b" },
      { sessions: [{ ...session, source_version: "b".repeat(64) }] },
    ]) {
      const choice = screen.getByRole("radio", { name: "Accept this request" });
      await waitFor(() => expect(choice).toBeEnabled());
      fireEvent.click(choice);
      current = { ...current, ...change };
      fireEvent.click(
        screen.getByRole("button", { name: "Refresh native approvals" }),
      );
      await waitFor(() =>
        expect(
          screen.getByRole("radio", { name: "Accept this request" }),
        ).not.toBeChecked(),
      );
    }
    offline = true;
    fireEvent.click(
      screen.getByRole("button", { name: "Refresh native approvals" }),
    );
    await screen.findByText(/Native connection or snapshot unavailable/);
    expect(
      screen.getByRole("radio", { name: "Accept this request" }),
    ).toBeDisabled();
    expect(screen.queryByText(/No approvals or warnings/)).toBeNull();
  });
});
