import { http, HttpResponse } from "msw";
import { setupListeners } from "@reduxjs/toolkit/query";
import { afterEach, describe, it, expect, vi } from "vitest";
import {
  act,
  render,
  screen,
  fireEvent,
  waitFor,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BuddyNativeApprovals } from "./BuddyNativeApprovals";
import { setUpStore } from "../../app/store";

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

const originalVisibilityState = Object.getOwnPropertyDescriptor(
  document,
  "visibilityState",
);
const originalHidden = Object.getOwnPropertyDescriptor(document, "hidden");

afterEach(() => {
  vi.useRealTimers();
  if (originalVisibilityState)
    Object.defineProperty(document, "visibilityState", originalVisibilityState);
  else Reflect.deleteProperty(document, "visibilityState");
  if (originalHidden) Object.defineProperty(document, "hidden", originalHidden);
  else Reflect.deleteProperty(document, "hidden");
});

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

  it("uses the joined chat title without exposing the chat id", async () => {
    const chatId = "53ee2bb5-b29a-4488-a69e-b1b63113101e";
    server.use(
      http.get("*/v1/buddy/approvals/current", () =>
        HttpResponse.json({
          ...snapshot,
          sessions: [{ ...session, chat_id: chatId }],
        }),
      ),
      http.get("*/v1/buddy/native-work", () =>
        HttpResponse.json({
          schema_version: 1,
          project_root: "/scratch",
          coverage: "loaded_native_sessions",
          next_cursor: null,
          sessions: [
            {
              chat_id: chatId,
              title: "Fix the garden path",
              model: "chosen/model",
              state: "waiting_user_input",
              persistence_pending: false,
              goal: null,
              agents: [],
              agents_truncated: false,
              pending_questions: 0,
              approval_count: 1,
              browser_decision_pending: false,
              error: null,
            },
          ],
        }),
      ),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    expect(await screen.findByText("Fix the garden path")).toBeTruthy();
    expect(screen.queryByText(chatId)).toBeNull();
    expect(screen.getByRole("group", { name: "Shell" })).toBeTruthy();
  });

  it("uses an untitled fallback when there is no matching nonblank title", async () => {
    server.use(
      http.get("*/v1/buddy/approvals/current", () =>
        HttpResponse.json({
          ...snapshot,
          sessions: [{ ...session, chat_id: "other-chat" }],
        }),
      ),
      http.get("*/v1/buddy/native-work", () =>
        HttpResponse.json({
          schema_version: 1,
          project_root: "/scratch",
          coverage: "loaded_native_sessions",
          next_cursor: null,
          sessions: [],
        }),
      ),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    expect(await screen.findByText("Untitled chat")).toBeTruthy();
  });

  it("does not borrow a title from another project root", async () => {
    server.use(
      http.get("*/v1/buddy/approvals/current", () =>
        HttpResponse.json({
          ...snapshot,
          sessions: [{ ...session, chat_id: "other-chat" }],
        }),
      ),
      http.get("*/v1/buddy/native-work", () =>
        HttpResponse.json({
          schema_version: 1,
          project_root: "/other",
          coverage: "loaded_native_sessions",
          next_cursor: null,
          sessions: [
            {
              chat_id: "other-chat",
              title: "Foreign project title",
              model: "chosen/model",
              state: "waiting_user_input",
              persistence_pending: false,
              goal: null,
              agents: [],
              agents_truncated: false,
              pending_questions: 0,
              approval_count: 1,
              browser_decision_pending: false,
              error: null,
            },
          ],
        }),
      ),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    expect(await screen.findByText("Untitled chat")).toBeTruthy();
    expect(screen.queryByText("Foreign project title")).toBeNull();
  });

  it("disambiguates repeated humanized tool labels without exposing call ids", async () => {
    server.use(
      http.get("*/v1/buddy/approvals/current", () =>
        HttpResponse.json({
          ...snapshot,
          sessions: [
            {
              ...session,
              tools: [
                session.tools[0],
                { ...session.tools[0], tool_call_id: "tool-2" },
              ],
            },
          ],
        }),
      ),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    expect(
      await screen.findByRole("group", { name: "Shell, request 1 of 2" }),
    ).toBeTruthy();
    expect(
      screen.getByRole("group", { name: "Shell, request 2 of 2" }),
    ).toBeTruthy();
    expect(screen.queryByText("tool-1")).toBeNull();
    expect(screen.queryByText("tool-2")).toBeNull();
  });

  it("explains a failed response without referring to the native chat", async () => {
    server.use(
      http.get("*/v1/buddy/approvals/current", () =>
        HttpResponse.json(snapshot),
      ),
      http.post("*/v1/buddy/approvals/chat-a/response", () =>
        HttpResponse.json({ error: "offline" }, { status: 503 }),
      ),
      http.get("*/v1/buddy/native-work", () =>
        HttpResponse.json({
          schema_version: 1,
          project_root: "/scratch",
          coverage: "loaded_native_sessions",
          next_cursor: null,
          sessions: [
            {
              chat_id: "chat-a",
              title: "Fix the garden path",
              model: "chosen/model",
              state: "waiting_user_input",
              persistence_pending: false,
              goal: null,
              agents: [],
              agents_truncated: false,
              pending_questions: 0,
              approval_count: 1,
              browser_decision_pending: false,
              error: null,
            },
          ],
        }),
      ),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("radio", { name: "Accept this request" }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Submit tool decision" }),
    );
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain('"Fix the garden path"');
    expect(alert.textContent?.toLowerCase()).not.toContain("the native chat");
    expect(screen.getByRole("button", { name: "Open chat" })).toBeTruthy();
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
    await screen.findByText(/Native approvals could not be loaded/);
    expect(
      screen.getByRole("radio", { name: "Accept this request" }),
    ).toBeDisabled();
    expect(screen.queryByText(/No approvals or warnings/)).toBeNull();
  });

  it("renders nothing at all when there is nothing to approve", async () => {
    let reads = 0;
    server.use(
      http.get("*/v1/buddy/approvals/current", () => {
        reads++;
        return HttpResponse.json({ ...snapshot, sessions: [] });
      }),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    await waitFor(() => expect(reads).toBeGreaterThan(0));
    expect(screen.queryByLabelText("Native approvals")).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Refresh native approvals" }),
    ).toBeNull();
  });

  it("shows an error with a retry instead of an empty state when the request fails", async () => {
    let reads = 0;
    let offline = true;
    server.use(
      http.get("*/v1/buddy/approvals/current", () => {
        reads++;
        return offline
          ? HttpResponse.json({ error: "mode down" }, { status: 503 })
          : HttpResponse.json({ ...snapshot, sessions: [] });
      }),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    expect(
      await screen.findByText(/Native approvals could not be loaded/),
    ).toBeTruthy();
    expect(screen.queryByText(/No approvals or warnings/)).toBeNull();
    const seen = reads;
    offline = false;
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() =>
      expect(screen.queryByLabelText("Native approvals")).toBeNull(),
    );
    expect(reads).toBeGreaterThan(seen);
  });

  it("marks a blocked request group as visibly disabled", async () => {
    server.use(
      http.get("*/v1/buddy/approvals/current", () =>
        HttpResponse.json(snapshot),
      ),
      http.post("*/v1/buddy/approvals/chat-a/response", () =>
        HttpResponse.json({ status: "queued" }, { status: 202 }),
      ),
    );
    render(<BuddyNativeApprovals />, { preloadedState: config });
    const choice = await screen.findByRole("radio", {
      name: "Accept this request",
    });
    await waitFor(() => expect(choice).toBeEnabled());
    const group = screen.getByRole("group", { name: "Shell" });
    expect(group).not.toHaveAttribute("data-disabled");
    fireEvent.click(choice);
    fireEvent.click(
      screen.getByRole("button", { name: "Submit tool decision" }),
    );
    await waitFor(() =>
      expect(screen.getByRole("group", { name: "Shell" })).toHaveAttribute(
        "data-disabled",
        "true",
      ),
    );
    expect(choice).toBeDisabled();
  });

  it("pauses approval polling while hidden and refetches on resume", async () => {
    let reads = 0;
    let visibility: DocumentVisibilityState = "visible";
    let hidden = false;
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => visibility,
    });
    Object.defineProperty(document, "hidden", {
      configurable: true,
      get: () => hidden,
    });
    server.use(
      http.get("*/v1/buddy/approvals/current", () => {
        reads += 1;
        return HttpResponse.json(snapshot);
      }),
    );
    vi.useFakeTimers();
    const store = setUpStore(config);
    const stopListeners = setupListeners(store.dispatch);
    const view = render(<BuddyNativeApprovals />, { store });
    try {
      await vi.waitFor(async () => {
        await act(async () => {
          await vi.advanceTimersByTimeAsync(10);
        });
        expect(reads).toBe(1);
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(12500);
      });
      const visibleReads = reads;
      expect(visibleReads).toBeGreaterThanOrEqual(3);

      act(() => {
        visibility = "hidden";
        hidden = true;
        document.dispatchEvent(new Event("visibilitychange"));
        window.dispatchEvent(new Event("visibilitychange"));
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      const hiddenReads = reads;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(15000);
      });
      expect(reads).toBe(hiddenReads);
      expect(hiddenReads).toBeLessThanOrEqual(visibleReads + 1);

      act(() => {
        visibility = "visible";
        hidden = false;
        document.dispatchEvent(new Event("visibilitychange"));
        window.dispatchEvent(new Event("visibilitychange"));
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(50);
      });
      expect(reads).toBe(visibleReads + 1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5000);
      });
      expect(reads).toBe(visibleReads + 2);
    } finally {
      stopListeners();
      view.unmount();
    }
  });
});
