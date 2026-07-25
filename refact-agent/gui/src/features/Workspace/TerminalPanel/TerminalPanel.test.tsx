import { readFileSync } from "node:fs";

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

const terminalPanelCss = readFileSync(
  "src/features/Workspace/TerminalPanel/TerminalPanel.module.css",
  "utf8",
);

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

  test("gives the expand-grid body an explicit tokenized block size", () => {
    const bodyRule = terminalPanelCss.match(/\.body\s*\{([^}]*)\}/u)?.[1];

    expect(bodyRule).toMatch(
      /block-size:\s*calc\(var\(--rf-control-h\)\s*\*\s*6\)/u,
    );
    expect(bodyRule).not.toMatch(/\bflex\s*:/u);
    expect(terminalPanelCss).toMatch(
      /\.body\[hidden\]\s*\{[^}]*display:\s*none/u,
    );
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

  test("closes transports while switching tabs and collapsing", async () => {
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
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    expect(
      container.querySelector('[data-terminal-process-id="first-123456"]'),
    ).toBeInTheDocument();
    expect(
      container.querySelector('[data-terminal-process-id="second-12345"]'),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: /\/bin\/zsh · second/i }));
    await waitFor(() =>
      expect(FakeEventSource.instances[0].close).toHaveBeenCalled(),
    );
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(2));
    expect(
      container.querySelector('[data-terminal-process-id="first-123456"]'),
    ).not.toBeInTheDocument();
    expect(
      container.querySelector('[data-terminal-process-id="second-12345"]'),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Collapse terminal workbench" }),
    );
    await waitFor(() =>
      expect(FakeEventSource.instances[1].close).toHaveBeenCalled(),
    );
    expect(container.querySelector("[data-terminal-process-id]")).toBeNull();
  });

  test("links tab panels and supports roving terminal tab keys", async () => {
    server.use(
      http.get("*/v1/exec/list", () =>
        HttpResponse.json({
          processes: ["first-123456", "second-12345", "third-123456"].map(
            (process_id) => ({
              process_id,
              status: "running",
              command_preview: "/bin/zsh",
              created_at_ms: 1,
              tty: true,
              service_name: null,
            }),
          ),
        }),
      ),
      http.get("*/v1/exec/:processId/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
      http.post("*/v1/exec/:processId/resize", () => HttpResponse.json({})),
    );

    const view = renderTerminalPanel();
    openWorkbench(view);
    const tabs = await screen.findAllByRole("tab");
    const first = tabs[0];
    const second = tabs[1];
    const third = tabs[2];

    expect(first).toHaveAttribute("tabindex", "0");
    expect(second).toHaveAttribute("tabindex", "-1");
    expect(
      document.getElementById(first.getAttribute("aria-controls") ?? ""),
    ).toHaveAttribute("role", "tabpanel");
    expect(first.id).toBe(
      document
        .getElementById(first.getAttribute("aria-controls") ?? "")
        ?.getAttribute("aria-labelledby"),
    );

    first.focus();
    await view.user.keyboard("{ArrowRight}");
    expect(second).toHaveFocus();
    expect(second).toHaveAttribute("aria-selected", "false");
    expect(first).toHaveAttribute("aria-selected", "true");
    expect(first).toHaveAttribute("tabindex", "-1");

    await view.user.keyboard("{End}");
    expect(third).toHaveFocus();
    await view.user.keyboard("{Home}");
    expect(first).toHaveFocus();
    await view.user.keyboard("{ArrowLeft}");
    expect(third).toHaveFocus();
    await view.user.keyboard("{Enter}");
    expect(third).toHaveAttribute("aria-selected", "true");
    expect(screen.getByLabelText("Terminal input")).toHaveFocus();
    await view.user.click(third);
    expect(screen.getByLabelText("Terminal input")).toHaveFocus();

    await view.user.click(
      screen.getByRole("button", { name: "Collapse terminal workbench" }),
    );
    await view.user.click(
      screen.getByRole("button", { name: "Expand terminal workbench" }),
    );
    expect(await screen.findByLabelText("Terminal input")).toHaveFocus();
  });

  test("reattaches every TTY status without opening hidden transports", async () => {
    server.use(
      http.get("*/v1/exec/list", () =>
        HttpResponse.json({
          processes: [
            {
              process_id: "starting-1234",
              status: "starting",
              command_preview: "starting-shell",
              created_at_ms: 1,
              tty: true,
              service_name: null,
            },
            {
              process_id: "exited-12345",
              status: "exited",
              command_preview: "finished-shell",
              created_at_ms: 2,
              tty: true,
              service_name: null,
            },
          ],
        }),
      ),
    );

    renderTerminalPanel();

    expect(
      await screen.findByRole("tab", { name: /starting-shell · starting/i }),
    ).toBeVisible();
    expect(
      screen.getByRole("tab", { name: /finished-shell · exited/i }),
    ).toBeVisible();
    expect(FakeEventSource.instances).toHaveLength(0);
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
    const killChatIds: (string | null)[] = [];
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
      http.post("*/v1/exec/spawned-1234/kill", ({ request }) => {
        killChatIds.push(new URL(request.url).searchParams.get("chat_id"));
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
    await waitFor(() => expect(killChatIds).toEqual(["chat-a"]));
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

  test("resets an initial list 403 when switching chats", async () => {
    const listChatIds: (string | null)[] = [];
    server.use(
      http.get("*/v1/exec/list", ({ request }) => {
        const chatId = new URL(request.url).searchParams.get("chat_id");
        listChatIds.push(chatId);
        return chatId === "chat-a"
          ? HttpResponse.text("forbidden", { status: 403 })
          : HttpResponse.json({ processes: [] });
      }),
    );

    const view = renderTerminalPanel();
    openWorkbench(view);
    expect(await screen.findByText("Browser terminal disabled")).toBeVisible();

    addChat(view, "chat-b");
    view.rerender(<TerminalPanel chatId="chat-b" />);
    openWorkbench(view, "chat-b");

    expect(await screen.findByText("No terminal sessions")).toBeVisible();
    expect(screen.queryByText("Browser terminal disabled")).toBeNull();
    expect(listChatIds).toEqual(expect.arrayContaining(["chat-a", "chat-b"]));
  });

  test("successful retry clears an initial list 403", async () => {
    let listCount = 0;
    server.use(
      http.get("*/v1/exec/list", () => {
        listCount += 1;
        return listCount === 1
          ? HttpResponse.text("forbidden", { status: 403 })
          : HttpResponse.json({ processes: [] });
      }),
    );

    const view = renderTerminalPanel();
    openWorkbench(view);
    await view.user.click(
      await screen.findByRole("button", { name: "Try again" }),
    );

    expect(await screen.findByText("No terminal sessions")).toBeVisible();
    expect(screen.queryByText("Browser terminal disabled")).toBeNull();
    expect(listCount).toBe(2);
  });
});
