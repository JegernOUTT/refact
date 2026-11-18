import { readFileSync } from "node:fs";

import { screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import { beforeEach, describe, expect, test, vi } from "vitest";

import type { ChatMessage } from "../../../services/refact/types";
import { render } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { applyChatEvent, createChatWithId } from "../../Chat/Thread";
import { makeSurfaceKey } from "../surfaceKey";
import {
  DRAWER_DEFAULT_HEIGHT,
  DRAWER_MIN_HEIGHT,
  openTab,
  selectWorkspaceDrawer,
  setDrawerHeight,
} from "../workspaceSlice";
import { TerminalPanel } from "./TerminalPanel";
import {
  selectActiveTerminalProcessId,
  selectTerminalSessions,
  selectTerminalWorkbenchOpen,
  setTerminalWorkbenchOpen,
} from "./terminalSlice";

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

function processCompleted(processId: string, description: string, seq: string) {
  return applyChatEvent({
    chat_id: "chat-a",
    seq,
    type: "message_added",
    index: Number(seq) - 2,
    message: {
      message_id: `completed-${processId}`,
      role: "event",
      content: `Background process '${description}' exited (exit 0)`,
      event: {
        subkind: "process_completed",
        source: "exec.registry",
        payload: {
          process_id: processId,
          status: "exited",
          exit_code: 0,
          duration_ms: 1234,
          short_description: description,
          mode: "background",
          push: "none",
        },
      },
    } as ChatMessage,
  });
}

describe("TerminalPanel", () => {
  beforeEach(() => {
    FakeEventSource.instances = [];
    vi.stubGlobal("EventSource", FakeEventSource);
    vi.spyOn(window, "confirm").mockReturnValue(true);
  });

  test("sizes the expand-grid body from the persisted drawer height", () => {
    const bodyRule = terminalPanelCss.match(/\.body\s*\{([^}]*)\}/u)?.[1];

    expect(bodyRule).toMatch(/block-size:\s*var\(--rf-terminal-h\)/u);
    expect(bodyRule).not.toMatch(/\bflex\s*:/u);
    expect(terminalPanelCss).toMatch(
      /\.body\[hidden\]\s*\{[^}]*display:\s*none/u,
    );
  });

  test("resizes the workbench from the separator and persists the height", async () => {
    server.use(
      http.get("*/v1/exec/list", () => HttpResponse.json({ processes: [] })),
    );
    const view = renderTerminalPanel();
    openWorkbench(view);

    const separator = await screen.findByRole("separator", {
      name: "Resize terminal",
    });
    const panel = screen.getByRole("region", {
      name: "Terminal workbench for chat-a",
    });
    expect(panel.style.getPropertyValue("--rf-terminal-h")).toBe(
      `${DRAWER_DEFAULT_HEIGHT}px`,
    );
    expect(separator).toHaveAttribute(
      "aria-valuenow",
      String(DRAWER_DEFAULT_HEIGHT),
    );

    separator.focus();
    await view.user.keyboard("{ArrowUp}");
    const grown = selectWorkspaceDrawer(view.store.getState()).height;
    expect(grown).toBeGreaterThan(DRAWER_DEFAULT_HEIGHT);
    expect(panel.style.getPropertyValue("--rf-terminal-h")).toBe(`${grown}px`);

    act(() => {
      view.store.dispatch(setDrawerHeight(DRAWER_MIN_HEIGHT));
    });
    await view.user.keyboard("{ArrowDown}");
    expect(selectWorkspaceDrawer(view.store.getState()).height).toBe(
      DRAWER_MIN_HEIGHT,
    );

    await view.user.click(screen.getByRole("button", { name: /^Terminal$/ }));
    expect(
      screen.queryByRole("separator", { name: "Resize terminal" }),
    ).toBeNull();
  });

  test("reattaches all process types and seeds backfill before streaming", async () => {
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
      http.get("*/v1/exec/background/read", () =>
        HttpResponse.json({
          chunks: [{ seq: 0, stream: "combined", text: "task output" }],
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
    const backgroundTab = screen.getByRole("tab", {
      name: /task · backgrou/i,
    });
    expect(backgroundTab).toBeVisible();
    await waitFor(() => expect(listChatIds).toContain("chat-a"));
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    expect(screen.queryByText("Read-only")).toBeNull();
    await view.user.click(backgroundTab);
    expect(screen.getByText("Read-only")).toBeVisible();
  });

  test("immediately closes completed process tabs, including the selected tab and a collapsed background tab", async () => {
    server.use(
      http.get("*/v1/exec/list", () =>
        HttpResponse.json({
          processes: [
            { process_id: "hidden-task", command_preview: "hidden task" },
            { process_id: "fallback-task", command_preview: "fallback task" },
          ].map((process) => ({
            ...process,
            status: "running",
            created_at_ms: 1,
            tty: false,
            service_name: null,
          })),
        }),
      ),
      http.get("*/v1/exec/:processId/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
    );
    const view = renderTerminalPanel();
    const fallback = await screen.findByRole("tab", { name: /fallback task/i });
    const hidden = screen.getByRole("tab", { name: /hidden task/i });

    view.store.dispatch(
      applyChatEvent({
        chat_id: "chat-a",
        seq: "1",
        type: "exec_process_spawned",
        process: {
          process_id: "tool-1234",
          command_preview: "git status",
          mode: "background",
          tty: false,
          status: "running",
          started_at: 1,
        },
      }),
    );

    const selected = await screen.findByRole("tab", {
      name: /git status · tool-123/i,
    });
    await view.user.click(selected);
    expect(selected).toHaveAttribute("aria-selected", "true");
    expect(selected).toHaveAttribute("aria-expanded", "true");
    expect(selected).toHaveTextContent("running");

    act(() => {
      view.store.dispatch(processCompleted("tool-1234", "git status", "2"));
      const state = view.store.getState();
      expect(
        selectTerminalSessions(state, "chat-a").map((s) => s.process_id),
      ).not.toContain("tool-1234");
      expect(selectActiveTerminalProcessId(state, "chat-a")).toBe(
        "fallback-task",
      );
    });
    expect(selected).not.toBeInTheDocument();
    expect(fallback).toHaveAttribute("aria-selected", "true");
    expect(fallback).toHaveAttribute("aria-expanded", "true");

    await view.user.click(screen.getByRole("button", { name: /^Terminal$/ }));
    expect(hidden).toHaveAttribute("aria-expanded", "false");
    expect(hidden).toHaveAttribute("aria-selected", "false");
    act(() => {
      view.store.dispatch(processCompleted("hidden-task", "hidden task", "3"));
    });
    expect(hidden).not.toBeInTheDocument();
    expect(fallback).toHaveAttribute("aria-selected", "true");
    expect(fallback).toHaveAttribute("aria-expanded", "false");
  });

  test("immediately closes a collapsed foreground tab from the tool result", async () => {
    server.use(
      http.get("*/v1/exec/list", () =>
        HttpResponse.json({
          processes: [
            {
              process_id: "foreground-task",
              status: "running",
              command_preview: "pwd",
              created_at_ms: 1,
              tty: false,
              service_name: null,
            },
          ],
        }),
      ),
    );
    const { store } = renderTerminalPanel();
    const tab = await screen.findByRole("tab", { name: /pwd · foregrou/i });
    expect(tab).toHaveAttribute("aria-expanded", "false");

    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: "chat-a",
          seq: "1",
          type: "message_added",
          index: 0,
          message: {
            message_id: "result-foreground-task",
            role: "tool",
            tool_call_id: "call-foreground-task",
            content: "/project",
            exec: {
              process_id: "foreground-task",
              status: "exited",
              mode: "foreground",
              exit_code: 0,
            },
          } as ChatMessage,
        }),
      );
      expect(selectTerminalSessions(store.getState(), "chat-a")).toEqual([]);
    });
    expect(tab).not.toBeInTheDocument();
  });

  test("keeps a live spawn when mount reconciliation resolves afterward", async () => {
    let resolveList: ((response: HttpResponse) => void) | undefined;
    server.use(
      http.get(
        "*/v1/exec/list",
        () =>
          new Promise<HttpResponse>((resolve) => {
            resolveList = resolve;
          }),
      ),
      http.get("*/v1/exec/quick-123/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
    );
    const view = renderTerminalPanel();
    openWorkbench(view);
    await waitFor(() => expect(resolveList).toBeDefined());

    view.store.dispatch(
      applyChatEvent({
        chat_id: "chat-a",
        seq: "1",
        type: "exec_process_spawned",
        process: {
          process_id: "quick-123",
          command_preview: "pwd",
          mode: "foreground",
          tty: false,
          status: "running",
          started_at: 1,
        },
      }),
    );
    await screen.findByRole("tab", { name: /pwd · quick-12/i });

    resolveList?.(HttpResponse.json({ processes: [] }));
    await waitFor(() =>
      expect(screen.queryByText("Finding terminal sessions…")).toBeNull(),
    );

    expect(screen.getByRole("tab", { name: /pwd · quick-12/i })).toBeVisible();
  });

  test.each(["live event", "snapshot"] as const)(
    "does not resurrect a completed process from a %s when a stale list resolves",
    async (source) => {
      let resolveList: ((response: HttpResponse) => void) | undefined;
      server.use(
        http.get(
          "*/v1/exec/list",
          () =>
            new Promise<HttpResponse>((resolve) => {
              resolveList = resolve;
            }),
        ),
      );
      const view = renderTerminalPanel();
      await waitFor(() => expect(resolveList).toBeDefined());

      const completion = processCompleted("stale-task", "stale task", "2");
      if (source === "live event") {
        view.store.dispatch(
          applyChatEvent({
            chat_id: "chat-a",
            seq: "1",
            type: "exec_process_spawned",
            process: {
              process_id: "stale-task",
              command_preview: "stale task",
              mode: "background",
              tty: false,
              status: "running",
              started_at: 1,
            },
          }),
        );
        await screen.findByRole("tab", { name: /stale task/i });
      }
      act(() => {
        if (source === "live event") {
          view.store.dispatch(completion);
          return;
        }
        if (completion.payload.type !== "message_added")
          throw new Error("Expected completion message");
        view.store.dispatch(
          applyChatEvent({
            chat_id: "chat-a",
            seq: "2",
            type: "snapshot",
            thread: {
              id: "chat-a",
              title: "Test",
              model: "test-model",
              mode: "AGENT",
              tool_use: "agent",
              boost_reasoning: false,
              context_tokens_cap: null,
              include_project_info: true,
              checkpoints_enabled: true,
              is_title_generated: false,
            },
            runtime: {
              state: "idle",
              paused: false,
              error: null,
              queue_size: 0,
              pause_reasons: [],
              queued_items: [],
            },
            messages: [completion.payload.message],
            background_agents: [],
          }),
        );
      });
      expect(screen.queryByRole("tab", { name: /stale task/i })).toBeNull();

      resolveList?.(
        HttpResponse.json({
          processes: [
            { process_id: "stale-task", command_preview: "stale task" },
            { process_id: "live-task", command_preview: "live task" },
          ].map((process) => ({
            ...process,
            status: "running",
            created_at_ms: 1,
            tty: false,
            service_name: null,
          })),
        }),
      );
      expect(
        await screen.findByRole("tab", { name: /live task/i }),
      ).toBeVisible();
      expect(screen.queryByRole("tab", { name: /stale task/i })).toBeNull();
    },
  );

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

    await user.click(screen.getByRole("tab", { name: /\/bin\/zsh · second/i }));
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
    expect(screen.queryByLabelText("Terminal input")).toBeNull();
    await view.user.click(third);
    expect(await screen.findByLabelText("Terminal input")).toHaveFocus();
  });

  test("reattaches only live list entries without opening hidden transports", async () => {
    const read = vi.fn(() =>
      HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
    );
    server.use(
      http.get("*/v1/exec/list", () =>
        HttpResponse.json({
          processes: [
            "starting",
            "running",
            "exited",
            "failed",
            "killed",
            "timed_out",
          ].map((status) => ({
            process_id: `${status}-1234`,
            status,
            command_preview: `${status}-shell`,
            created_at_ms: 1,
            tty: true,
            service_name: null,
          })),
        }),
      ),
      http.get("*/v1/exec/:processId/read", read),
    );

    renderTerminalPanel();

    expect(
      await screen.findByRole("tab", { name: /starting-shell · starting/i }),
    ).toBeVisible();
    expect(screen.getByRole("tab", { name: /running-shell/i })).toBeVisible();
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    expect(read).not.toHaveBeenCalled();
    expect(FakeEventSource.instances).toHaveLength(0);
  });

  test("immediately closes an interactive tab on exit SSE", async () => {
    server.use(
      http.get("*/v1/exec/list", () =>
        HttpResponse.json({
          processes: [
            {
              process_id: "live-shell-1",
              status: "running",
              command_preview: "/bin/zsh",
              created_at_ms: 1,
              tty: true,
              service_name: null,
            },
          ],
        }),
      ),
      http.get("*/v1/exec/live-shell-1/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
      http.post("*/v1/exec/live-shell-1/resize", () => HttpResponse.json({})),
    );

    const view = renderTerminalPanel();
    openWorkbench(view);
    const tab = await screen.findByRole("tab", { name: /\/bin\/zsh/i });
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    const source = FakeEventSource.instances[0];

    act(() => {
      source.emit("exit", {
        process_id: "live-shell-1",
        status: "exited",
        exit_code: 0,
        ended_at_ms: Date.now(),
      });
    });

    expect(tab).not.toBeInTheDocument();
    expect(screen.getByText("No terminal sessions")).toBeVisible();
    expect(source.close).toHaveBeenCalled();
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

    const toggle = screen.getByRole("button", { name: /^Terminal$/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.queryByRole("heading", { name: "No terminal sessions" }),
    ).toBeNull();

    await user.click(toggle);

    const emptyTitle = await screen.findByText("No terminal sessions");
    expect(emptyTitle).toBeVisible();

    await user.click(toggle);

    expect(
      screen.queryByRole("heading", { name: "No terminal sessions" }),
    ).toBeNull();
    expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  describe("tab toggling", () => {
    function useTwoSessions() {
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
    }

    test("a tab expands a collapsed panel and activates itself", async () => {
      useTwoSessions();
      const { user, store } = renderTerminalPanel();
      const second = await screen.findByRole("tab", { name: /second/i });
      expect(second).toHaveAttribute("aria-expanded", "false");

      await user.click(second);

      expect(second).toHaveAttribute("aria-expanded", "true");
      expect(second).toHaveAttribute("aria-selected", "true");
      expect(selectTerminalWorkbenchOpen(store.getState(), "chat-a")).toBe(
        true,
      );
    });

    test("double-clicking a tab opens the panel once", async () => {
      useTwoSessions();
      const { user, store } = renderTerminalPanel();
      const second = await screen.findByRole("tab", { name: /second/i });

      await user.dblClick(second);

      expect(second).toHaveAttribute("aria-expanded", "true");
      expect(second).toHaveAttribute("aria-selected", "true");
      expect(selectTerminalWorkbenchOpen(store.getState(), "chat-a")).toBe(
        true,
      );
    });

    test("the active tab collapses an expanded panel", async () => {
      useTwoSessions();
      const view = renderTerminalPanel();
      openWorkbench(view);
      const first = await screen.findByRole("tab", { name: /first/i });
      expect(first).toHaveAttribute("aria-selected", "true");
      expect(first).toHaveAttribute("aria-expanded", "true");

      await view.user.click(first);

      expect(first).toHaveAttribute("aria-expanded", "false");
      expect(first).toHaveAttribute("aria-selected", "true");
      expect(screen.queryByRole("separator")).toBeNull();
    });

    test("another tab switches and keeps the panel expanded", async () => {
      useTwoSessions();
      const view = renderTerminalPanel();
      openWorkbench(view);
      const first = await screen.findByRole("tab", { name: /first/i });
      const second = screen.getByRole("tab", { name: /second/i });

      await view.user.click(second);

      expect(second).toHaveAttribute("aria-selected", "true");
      expect(first).toHaveAttribute("aria-selected", "false");
      expect(second).toHaveAttribute("aria-expanded", "true");
      expect(
        screen.getByRole("separator", { name: "Resize terminal" }),
      ).toBeVisible();
    });

    test("Enter and Space on a tab toggle the panel", async () => {
      useTwoSessions();
      const view = renderTerminalPanel();
      openWorkbench(view);
      const first = await screen.findByRole("tab", { name: /first/i });

      first.focus();
      await view.user.keyboard("{Enter}");
      expect(first).toHaveAttribute("aria-expanded", "false");
      await view.user.keyboard(" ");
      expect(first).toHaveAttribute("aria-expanded", "true");
    });

    test("closing a tab does not toggle the panel", async () => {
      useTwoSessions();
      server.use(
        http.post("*/v1/exec/:processId/kill", () => HttpResponse.json({})),
      );
      const view = renderTerminalPanel();
      openWorkbench(view);
      const first = await screen.findByRole("tab", { name: /first/i });

      await view.user.click(
        screen.getByRole("button", { name: /^Close .*first/i }),
      );

      await waitFor(() =>
        expect(screen.queryByRole("tab", { name: /first/i })).toBeNull(),
      );
      expect(screen.getByRole("tab", { name: /second/i })).toHaveAttribute(
        "aria-expanded",
        "true",
      );
      expect(first).not.toBeInTheDocument();
    });

    test("the Terminal title toggles the panel and exposes its state", async () => {
      useTwoSessions();
      const { user } = renderTerminalPanel();
      const title = screen.getByRole("button", { name: /^Terminal$/ });
      expect(title).toHaveAttribute("aria-expanded", "false");

      await user.click(title);
      expect(title).toHaveAttribute("aria-expanded", "true");
      await user.click(title);
      expect(title).toHaveAttribute("aria-expanded", "false");
    });

    test("New terminal expands a collapsed panel", async () => {
      useTwoSessions();
      server.use(
        http.post("*/v1/exec/spawn", () =>
          HttpResponse.json({
            process_id: "spawned-1234",
            status: "running",
            command_preview: "/bin/zsh",
          }),
        ),
      );
      const { user } = renderTerminalPanel();
      await screen.findByRole("tab", { name: /first/i });
      expect(screen.queryByRole("separator")).toBeNull();

      await user.click(screen.getByRole("button", { name: "New terminal" }));

      expect(
        await screen.findByRole("tab", { name: /spawned/i }),
      ).toHaveAttribute("aria-expanded", "true");
      expect(
        screen.getByRole("separator", { name: "Resize terminal" }),
      ).toBeVisible();
    });
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
        env: { TERM: "xterm-256color", COLORTERM: "truecolor", NO_COLOR: "" },
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

    const disabledTitle = await screen.findByText("Browser terminal disabled");
    expect(disabledTitle).toBeVisible();
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
