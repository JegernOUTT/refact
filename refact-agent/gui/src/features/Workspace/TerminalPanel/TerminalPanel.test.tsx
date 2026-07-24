import { screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, test, vi } from "vitest";

import { render } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { createChatWithId } from "../../Chat/Thread";
import { makeSurfaceKey } from "../surfaceKey";
import { openTab } from "../workspaceSlice";
import { TerminalPanel } from "./TerminalPanel";
import { setTerminalWorkbenchOpen } from "./terminalSlice";

class FakeEventSource {
  static instances: FakeEventSource[] = [];

  onopen: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  close = vi.fn();
  private readonly listeners = new Map<string, EventListener[]>();

  constructor(_url: string | URL) {
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: EventListener) {
    const listeners = this.listeners.get(type) ?? [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }

  emit(type: string, data: unknown) {
    const event = new MessageEvent(type, { data: JSON.stringify(data) });
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }
}

const CONFIG_STATE = {
  config: {
    host: "web" as const,
    lspPort: 8001,
    apiKey: null,
    themeProps: {},
  },
};

function renderTerminalPanel(chatId = "chat-a") {
  const view = render(<TerminalPanel chatId={chatId} />, {
    preloadedState: {
      ...CONFIG_STATE,
      current_project: {
        name: "workspace",
        workspaceRoots: ["/project"],
      },
    },
  });
  view.store.dispatch(
    createChatWithId({
      id: "chat-a",
      worktree: {
        id: "worktree-a",
        kind: "task_agent",
        root: "/worktrees/chat-a",
        source_workspace_root: "/project",
        repo_root: "/project",
        enforce: true,
      },
    }),
  );
  view.store.dispatch(openTab(makeSurfaceKey("chat", "chat-a")));
  return view;
}

function addChat(view: ReturnType<typeof renderTerminalPanel>, id: string) {
  view.store.dispatch(
    createChatWithId({
      id,
      worktree: {
        id: `worktree-${id}`,
        kind: "task_agent",
        root: `/worktrees/${id}`,
        source_workspace_root: "/project",
        repo_root: "/project",
        enforce: true,
      },
    }),
  );
  view.store.dispatch(openTab(makeSurfaceKey("chat", id)));
}

function openWorkbench(
  view: ReturnType<typeof renderTerminalPanel>,
  chatId = "chat-a",
) {
  view.store.dispatch(setTerminalWorkbenchOpen({ chatId, open: true }));
}

describe("TerminalPanel", () => {
  beforeEach(() => {
    FakeEventSource.instances = [];
    vi.stubGlobal("EventSource", FakeEventSource);
    vi.spyOn(window, "confirm").mockReturnValue(true);
  });

  test("reattaches running PTYs and seeds backfill before streaming", async () => {
    const listChatIds: (string | null)[] = [];
    server.use(
      http.get("*/v1/exec/list", ({ request }) => {
        listChatIds.push(new URL(request.url).searchParams.get("chat_id"));
        return HttpResponse.json({
          processes: [
            {
              process_id: "reattach-123456",
              status: "running",
              command_preview: "/bin/zsh",
              created_at_ms: 1,
              tty: true,
              service_name: null,
            },
            {
              process_id: "background",
              status: "running",
              command_preview: "task",
              created_at_ms: 2,
              tty: false,
              service_name: null,
            },
          ],
        });
      }),
      http.get("*/v1/exec/reattach-123456/read", () =>
        HttpResponse.json({
          chunks: [{ seq: 0, stream: "combined", text: "history" }],
          next_seq: 1,
          status: "running",
        }),
      ),
      http.post("*/v1/exec/reattach-123456/resize", () =>
        HttpResponse.json({}),
      ),
    );

    const view = renderTerminalPanel();
    openWorkbench(view);

    expect(
      await screen.findByRole("tab", { name: /\/bin\/zsh · reattach/i }),
    ).toBeVisible();
    await waitFor(() => expect(listChatIds).toContain("chat-a"));
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    expect(screen.queryByText("background")).not.toBeInTheDocument();
  });

  test("keeps terminal sessions mounted while switching internal tabs", async () => {
    server.use(
      http.get("*/v1/exec/list", () =>
        HttpResponse.json({
          processes: ["first-123456", "second-12345"].map((process_id) => ({
            process_id,
            status: "running",
            command_preview: "/bin/zsh",
            created_at_ms: 1,
            tty: true,
            service_name: null,
          })),
        }),
      ),
      http.get("*/v1/exec/:processId/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
      http.post("*/v1/exec/:processId/resize", () => HttpResponse.json({})),
    );

    const view = renderTerminalPanel();
    openWorkbench(view);
    const { container, user } = view;
    await screen.findByRole("tab", { name: /\/bin\/zsh · first/i });
    const first = container.querySelector(
      '[data-terminal-process-id="first-123456"]',
    );
    const second = container.querySelector(
      '[data-terminal-process-id="second-12345"]',
    );
    expect(first).toBeInTheDocument();
    expect(second).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: /\/bin\/zsh · first/i }));
    expect(
      container.querySelector('[data-terminal-process-id="first-123456"]'),
    ).toBe(first);
    expect(
      container.querySelector('[data-terminal-process-id="second-12345"]'),
    ).toBe(second);
  });

  test("keeps two explicit chat workbenches isolated", async () => {
    const listChatIds: (string | null)[] = [];
    server.use(
      http.get("*/v1/exec/list", ({ request }) => {
        const chatId = new URL(request.url).searchParams.get("chat_id");
        listChatIds.push(chatId);
        return HttpResponse.json({
          processes: chatId
            ? [
                {
                  process_id: `${chatId}-process`,
                  status: "running",
                  command_preview: chatId,
                  created_at_ms: 1,
                  tty: true,
                  service_name: null,
                },
              ]
            : [],
        });
      }),
      http.get("*/v1/exec/:processId/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
      http.post("*/v1/exec/:processId/resize", () => HttpResponse.json({})),
    );

    const view = renderTerminalPanel("chat-a");
    addChat(view, "chat-b");
    view.rerender(
      <>
        <TerminalPanel chatId="chat-a" />
        <TerminalPanel chatId="chat-b" />
      </>,
    );
    openWorkbench(view, "chat-a");
    openWorkbench(view, "chat-b");

    expect(
      await screen.findByRole("tab", { name: /chat-b · chat-b-p/i }),
    ).toBeVisible();
    expect(
      await screen.findByRole("tab", { name: /chat-a · chat-a-p/i }),
    ).toBeVisible();
    expect(screen.getAllByLabelText("Terminal sessions")).toHaveLength(2);
    expect(listChatIds).toEqual(expect.arrayContaining(["chat-a", "chat-b"]));
  });

  test("keeps the collapsed body out of the accessibility tree", async () => {
    server.use(
      http.get("*/v1/exec/list", () => HttpResponse.json({ processes: [] })),
    );

    const { user } = renderTerminalPanel();

    expect(
      screen.getByRole("button", { name: "Expand terminal workbench" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "No terminal sessions" }),
    ).toBeNull();

    await user.click(
      screen.getByRole("button", { name: "Expand terminal workbench" }),
    );

    expect(await screen.findByText("No terminal sessions")).toBeVisible();

    await user.click(
      screen.getByRole("button", { name: "Collapse terminal workbench" }),
    );

    expect(
      screen.queryByRole("heading", { name: "No terminal sessions" }),
    ).toBeNull();
    expect(
      screen.getByRole("button", { name: "Expand terminal workbench" }),
    ).toBeVisible();
  });

  test("spawns the backend-selected shell and kills it when closed", async () => {
    const spawnBodies: unknown[] = [];
    let killed = false;
    server.use(
      http.get("*/v1/exec/list", () => HttpResponse.json({ processes: [] })),
      http.post("*/v1/exec/spawn", async ({ request }) => {
        spawnBodies.push(await request.json());
        return HttpResponse.json({
          process_id: "spawned-1234",
          status: "running",
          command_preview: "powershell.exe",
        });
      }),
      http.get("*/v1/exec/spawned-1234/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
      http.post("*/v1/exec/spawned-1234/resize", () => HttpResponse.json({})),
      http.post("*/v1/exec/spawned-1234/kill", () => {
        killed = true;
        return HttpResponse.json({
          process_id: "spawned-1234",
          status: "killed",
        });
      }),
    );

    const { user } = renderTerminalPanel();
    await user.click(
      await screen.findByRole("button", { name: "New terminal" }),
    );

    expect(
      await screen.findByRole("tab", { name: /powershell\.exe · spawned/i }),
    ).toBeVisible();
    expect(spawnBodies).toEqual([
      {
        chat_id: "chat-a",
        cwd: "/worktrees/chat-a",
        pty: true,
        rows: 24,
        cols: 80,
      },
    ]);

    await user.click(
      screen.getByRole("button", { name: /Close powershell\.exe · spawned/i }),
    );
    await waitFor(() => expect(killed).toBe(true));
    expect(window.confirm).toHaveBeenCalled();
    await waitFor(() =>
      expect(
        screen.queryByRole("tab", { name: /powershell\.exe · spawned/i }),
      ).not.toBeInTheDocument(),
    );
  });

  test("uses the shell fallback when spawn preview is absent", async () => {
    server.use(
      http.get("*/v1/exec/list", () => HttpResponse.json({ processes: [] })),
      http.post("*/v1/exec/spawn", () =>
        HttpResponse.json({ process_id: "fallback-1234", status: "running" }),
      ),
      http.get("*/v1/exec/fallback-1234/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
      http.post("*/v1/exec/fallback-1234/resize", () => HttpResponse.json({})),
    );

    const { user } = renderTerminalPanel();
    await user.click(
      await screen.findByRole("button", { name: "New terminal" }),
    );

    expect(
      await screen.findByRole("tab", { name: /shell · fallback/i }),
    ).toBeVisible();
  });

  test("shows an honest disabled state for a 403 spawn response", async () => {
    server.use(
      http.get("*/v1/exec/list", () => HttpResponse.json({ processes: [] })),
      http.post("*/v1/exec/spawn", () =>
        HttpResponse.text("exec HTTP is disabled", { status: 403 }),
      ),
    );

    const { user } = renderTerminalPanel();
    await user.click(
      await screen.findByRole("button", { name: "New terminal" }),
    );

    expect(await screen.findByText("Browser terminal disabled")).toBeVisible();
    expect(screen.getByText(/REFACT_DISABLE_EXEC_HTTP policy/i)).toBeVisible();
  });
});
