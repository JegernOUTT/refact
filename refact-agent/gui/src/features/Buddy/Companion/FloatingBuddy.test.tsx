import { beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  createDefaultChatState,
} from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { createChatWithId, switchToThread } from "../../Chat/Thread";
import { registerVisibleChatMount } from "../../Connection";
import { updateConfig } from "../../Config/configSlice";
import { act } from "react-dom/test-utils";
import { FloatingBuddy } from "./FloatingBuddy";
import type { AttentionSnapshot } from "../../../services/refact/buddyCompanion";

beforeEach(() => {
  localStorage.clear();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
});

vi.mock("../../../hooks/useInternalNavigation", () => ({
  useInternalNavigation: () => ({ navigate: vi.fn(), error: null }),
}));
const snapshot: AttentionSnapshot = {
  schema_version: 1,
  project_root: "/project",
  engine_identity: "engine",
  coverage: "loaded_sessions",
  partial: true,
  busy_chat_ids: [],
  rows: [
    {
      id: "request-a",
      kind: "tool_permission",
      chat_id: "source-b",
      source_version: "v1",
      reason: "Permission required",
      effect: "execute_pending_tool",
      actionable: true,
      literal_command: "echo hello",
      display_command: "shell echo hello",
      cwd: null,
      tool_call_id: "tool-a",
      question: null,
      parent_chat_id: null,
      run_generation: null,
      expires_at: null,
      tool_name: "shell",
      tool_arguments: { command: "echo hello" },
    },
  ],
};
function state() {
  const chat = createDefaultChatState();
  const id = chat.current_thread_id;
  return {
    id,
    preloadedState: {
      chat,
      pages: [{ name: "chat" as const }],
      connection: {
        browserOnline: true,
        backendStatus: "online" as const,
        backendLastOkAt: null,
        backendError: null,
        sseConnections: {},
        visibleChatMounts: { [id]: 1 },
      },
      config: {
        apiKey: "test",
        lspPort: 8001,
        themeProps: {},
        host: "vscode" as const,
      },
    },
  };
}
describe("App-root floating Buddy", () => {
  it("keeps main draft and source selected when incoming compact permission is approved", async () => {
    const calls: unknown[] = [];
    server.use(
      http.get("*/v1/buddy/attention", () => HttpResponse.json(snapshot)),
      http.post(
        "*/v1/buddy/approvals/source-b/response",
        async ({ request }) => {
          calls.push(await request.json());
          return HttpResponse.json({ queued: true }, { status: 202 });
        },
      ),
      http.get(
        "*/v1/buddy/approvals/source-b/outcomes/:id",
        () => new HttpResponse(null, { status: 503 }),
      ),
    );
    const source = state();
    const { store } = render(
      <>
        <textarea
          aria-label="Main draft"
          defaultValue="Unfinished source A question"
        />
        <FloatingBuddy />
      </>,
      { preloadedState: source.preloadedState },
    );
    const draft = screen.getByRole("textbox", { name: "Main draft" });
    draft.focus();
    await screen.findByLabelText("Buddy attention");
    expect(document.activeElement).toBe(draft);
    expect(draft).toHaveValue("Unfinished source A question");
    act(() => {
      fireEvent.blur(draft);
      draft.blur();
    });
    const approve = await screen.findByRole("button", { name: "Approve once" });
    fireEvent.click(approve);
    await waitFor(() => expect(calls).toHaveLength(1));
    expect(store.getState().chat.current_thread_id).toBe(source.id);
    expect(draft).toHaveValue("Unfinished source A question");
    expect(screen.queryByRole("dialog")).toBeNull();
  });
  it("opens the focused visible IDE source without creating a companion until first send", async () => {
    const creates = vi.fn();
    server.use(
      http.get("*/v1/buddy/attention", () =>
        HttpResponse.json({ ...snapshot, rows: [] }),
      ),
      http.post("*/v1/buddy/companion/:id", () => {
        creates();
        return new HttpResponse(null, { status: 503 });
      }),
    );
    const source = state();
    const { user } = render(<FloatingBuddy />, {
      preloadedState: source.preloadedState,
    });
    await user.click(screen.getByRole("button", { name: "Ask Buddy" }));
    const input = await screen.findByRole("textbox", {
      name: "Ask Buddy about the selected source chat",
    });
    expect(creates).not.toHaveBeenCalled();
    await user.type(input, "What is happening?");
    await user.click(screen.getByRole("button", { name: "Send to Buddy" }));
    await waitFor(() => expect(creates).toHaveBeenCalledTimes(1));
    expect(input).toHaveValue("What is happening?");
  });
  it("keeps late companion creation bound to A after switching B, and ignores old authorization results", async () => {
    let release: (() => void) | undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let requested = "";
    server.use(
      http.get("*/v1/buddy/attention", () =>
        HttpResponse.json({ ...snapshot, rows: [] }),
      ),
      http.post("*/v1/buddy/companion/:id", async ({ params }) => {
        requested = String(params.id);
        await gate;
        return HttpResponse.json({
          chat_id: "buddy-old-auth",
          source_chat_id: requested,
          title: "Buddy for A",
          kind: "companion",
          created_at: "now",
        });
      }),
    );
    const source = state();
    const { user, store } = render(<FloatingBuddy />, {
      preloadedState: source.preloadedState,
    });
    await user.click(screen.getByRole("button", { name: "Ask Buddy" }));
    await user.type(
      await screen.findByRole("textbox", {
        name: "Ask Buddy about the selected source chat",
      }),
      "A private draft",
    );
    await user.click(screen.getByRole("button", { name: "Send to Buddy" }));
    await waitFor(() => expect(requested).toBe(source.id));
    act(() => {
      store.dispatch(
        createChatWithId({
          id: "source-b",
          title: "B",
          activate: false,
          openTab: false,
        }),
      );
      store.dispatch(registerVisibleChatMount({ chatId: "source-b" }));
      store.dispatch(switchToThread({ id: "source-b", openTab: false }));
    });
    expect(
      screen.getByRole("textbox", {
        name: "Ask Buddy about the selected source chat",
      }),
    ).toHaveValue("");
    act(() => {
      store.dispatch(updateConfig({ apiKey: "different-user" }));
    });
    await act(async () => {
      release?.();
      await gate;
    });
    await waitFor(() =>
      expect(store.getState().chat.current_thread_id).toBe("source-b"),
    );
    expect(store.getState().chat.threads["buddy-old-auth"]).toBeUndefined();
  });
  it("recovers a submitted decision after app remount and polls the original id without another POST", async () => {
    let posts = 0;
    const receipts: string[] = [];
    server.use(
      http.get("*/v1/buddy/attention", () => HttpResponse.json(snapshot)),
      http.post("*/v1/buddy/approvals/source-b/response", () => {
        posts += 1;
        return HttpResponse.json({}, { status: 202 });
      }),
      http.get("*/v1/buddy/approvals/source-b/outcomes/:id", ({ params }) => {
        receipts.push(String(params.id));
        return new HttpResponse(null, { status: 503 });
      }),
    );
    const source = state();
    const first = render(<FloatingBuddy />, {
      preloadedState: source.preloadedState,
    });
    fireEvent.click(
      await screen.findByRole("button", { name: "Approve once" }),
    );
    await waitFor(() => expect(posts).toBe(1));
    await waitFor(() => expect(receipts.length).toBeGreaterThan(0));
    const original = receipts[0];
    first.unmount();
    render(<FloatingBuddy />, { preloadedState: source.preloadedState });
    await waitFor(() => expect(receipts.length).toBeGreaterThan(1));
    expect(screen.getByRole("button", { name: "Approve once" })).toBeDisabled();
    expect(receipts.every((id) => id === original)).toBe(true);
    expect(posts).toBe(1);
    expect(JSON.stringify(Object.values(localStorage))).not.toContain(
      "echo hello",
    );
  });
});
