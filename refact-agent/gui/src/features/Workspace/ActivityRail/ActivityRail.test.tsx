import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";

import type { BackgroundAgentSummary } from "../../../services/refact";
import { type AppStore, setUpStore } from "../../../app/store";
import { server } from "../../../utils/mockServer";
import {
  createDefaultChatState,
  render,
  screen,
  waitFor,
} from "../../../utils/test-utils";
import { updateConfig } from "../../Config/configSlice";
import { setBrowserOnline } from "../../Connection/connectionSlice";
import { push } from "../../Pages/pagesSlice";
import { collectTabIds } from "../../ChatPanes/panesTree";
import { Toolbar } from "../../../components/Toolbar/Toolbar";
import { createChatWithId, switchToThread } from "../../Chat/Thread";
import {
  openTab,
  setActiveTab,
  setDockOpen,
  setDockSection,
  setPanelsForced,
} from "../workspaceSlice";
import { makeSurfaceKey } from "../surfaceKey";
import { ActivityRail } from "./ActivityRail";
import styles from "./ActivityRail.module.css";

const chatId = "chat-a";

function agent(
  id: string,
  overrides: Partial<BackgroundAgentSummary> = {},
): BackgroundAgentSummary {
  return {
    agent_id: id,
    parent_chat_id: chatId,
    child_chat_id: `${id}-child`,
    kind: "subagent",
    status: "running",
    title: id,
    progress: null,
    step_count: 0,
    last_activity: "2026-08-31T10:00:00Z",
    target_files: [],
    edited_files: [],
    diff_summary: null,
    conflict_summary: null,
    result_summary: null,
    error: null,
    started_at: null,
    finished_at: null,
    change_seq: 1,
    ...overrides,
  };
}

function chatStateWithAgents(agents: BackgroundAgentSummary[]) {
  const chat = createDefaultChatState();
  const [sourceRuntime] = Object.values(chat.threads);
  chat.current_thread_id = chatId;
  chat.open_thread_ids = [chatId];
  chat.threads = {
    [chatId]: {
      ...sourceRuntime,
      thread: { ...sourceRuntime.thread, id: chatId },
      background_agents: Object.fromEntries(
        agents.map((item) => [item.agent_id, item]),
      ),
    },
  };
  return chat;
}

function createRailStore(agents: BackgroundAgentSummary[] = []): AppStore {
  const store = setUpStore({ chat: chatStateWithAgents(agents) });
  store.dispatch(createChatWithId({ id: chatId, title: "Chat Alpha" }));
  store.dispatch(openTab(makeSurfaceKey("chat", chatId)));
  store.dispatch(setActiveTab(makeSurfaceKey("chat", chatId)));
  return store;
}

function renderRail(store: AppStore) {
  return render(<ActivityRail />, { store });
}

describe("ActivityRail", () => {
  beforeEach(() => {
    server.use(
      http.get("*/v1/git/status", () => HttpResponse.json({ roots: [] })),
    );
  });

  it("renders exactly one Home at the rail top alongside the toolbar", () => {
    const store = createRailStore();
    store.dispatch(push({ name: "history" }));
    render(
      <>
        <Toolbar activeTab={{ type: "dashboard" }} />
        <ActivityRail />
      </>,
      { store },
    );

    const home = screen.getByRole("button", { name: "Home" });
    const rail = screen.getByRole("navigation", { name: "Workspace sections" });
    expect(rail.querySelector("button")).toBe(home);
    expect(home).toHaveAttribute("aria-pressed", "true");
    expect(home).toHaveClass(styles.button);
    const logo = home.querySelector("svg");
    expect(logo).toHaveAttribute("viewBox", "200 180 400 480");
    expect(logo).toHaveAttribute("aria-hidden", "true");
    expect(logo).toHaveAttribute("focusable", "false");
    expect(logo).toHaveClass(styles.homeLogo, "rf-status-pulse");
  });

  it("keeps the Home tooltip and keyboard navigation with the brand glyph", async () => {
    const store = createRailStore();
    store.dispatch(push({ name: "chat" }));
    const view = renderRail(store);
    const home = screen.getByRole("button", { name: "Home" });

    await view.user.hover(home);
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Home");
    home.focus();
    await view.user.keyboard("{Enter}");

    expect(store.getState().pages.at(-1)?.name).toBe("history");
    expect(store.getState().chat.threads[chatId]).toBeUndefined();
    expect(home).toHaveFocus();
  });

  it("returns to existing history, cleans only the focused empty chat, and leaves dock closed", async () => {
    const store = createRailStore();
    store.dispatch(
      createChatWithId({ id: "hidden", title: "Hidden", openTab: false }),
    );
    store.dispatch(switchToThread({ id: "hidden", openTab: false }));
    store.dispatch(push({ name: "history" }));
    store.dispatch(push({ name: "chat" }));
    store.dispatch(setDockOpen(false));
    const view = renderRail(store);
    const home = screen.getByRole("button", { name: "Home" });
    expect(home).toHaveAttribute("aria-pressed", "false");

    await view.user.click(home);

    expect(store.getState().pages).toEqual([
      { name: "login page" },
      { name: "history" },
    ]);
    expect(store.getState().chat.threads[chatId]).toBeUndefined();
    expect(store.getState().chat.threads.hidden).toBeDefined();
    expect(store.getState().workspace.dock?.open).toBe(false);
    expect(home).toHaveAttribute("aria-pressed", "true");
  });

  it("preserves empty chats with open local files on Home", async () => {
    const store = createRailStore();
    store.dispatch(openTab(makeSurfaceKey("file", "/repo/local.ts")));
    store.dispatch(setActiveTab(makeSurfaceKey("chat", chatId)));
    store.dispatch(push({ name: "chat" }));
    const view = renderRail(store);

    await view.user.click(screen.getByRole("button", { name: "Home" }));

    expect(store.getState().chat.threads[chatId]).toBeDefined();
    expect(
      Object.values(store.getState().workspace.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).toContain(makeSurfaceKey("file", "/repo/local.ts"));
    expect(store.getState().pages.at(-1)?.name).toBe("history");
  });

  it.each(["buddy", "task workspace", "history"] as const)(
    "does not clean the focused chat when Home is clicked from %s",
    async (name) => {
      const store = createRailStore();
      store.dispatch(
        push(name === "task workspace" ? { name, taskId: "task-a" } : { name }),
      );
      const view = renderRail(store);

      await view.user.click(screen.getByRole("button", { name: "Home" }));

      expect(store.getState().chat.threads[chatId]).toBeDefined();
      expect(store.getState().pages.at(-1)?.name).toBe("history");
    },
  );

  it("marks only the open dock section as pressed", () => {
    const store = createRailStore();
    store.dispatch(setDockSection("files"));
    store.dispatch(setDockOpen(true));
    renderRail(store);

    expect(screen.getByRole("button", { name: "Files" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Git" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("marks nothing as pressed when the dock is closed", () => {
    const store = createRailStore();
    store.dispatch(setDockSection("files"));
    store.dispatch(setDockOpen(false));
    renderRail(store);

    for (const label of ["Files", "Git", "Agents", "Active chats"]) {
      expect(screen.getByRole("button", { name: label })).toHaveAttribute(
        "aria-pressed",
        "false",
      );
    }
  });

  it("selects a section and opens the dock on click", async () => {
    const store = createRailStore();
    store.dispatch(setDockSection("files"));
    store.dispatch(setDockOpen(false));
    const view = renderRail(store);

    await view.user.click(screen.getByRole("button", { name: "Git" }));

    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "git",
    });
  });

  it("collapses the dock when the active section is clicked again", async () => {
    const store = createRailStore();
    store.dispatch(setDockSection("git"));
    store.dispatch(setDockOpen(true));
    const view = renderRail(store);

    await view.user.click(screen.getByRole("button", { name: "Git" }));

    expect(store.getState().workspace.dock?.open).toBe(false);
    expect(store.getState().workspace.dock?.section).toBe("git");
  });

  it("switches sections without closing when a different section is clicked", async () => {
    const store = createRailStore();
    store.dispatch(setDockSection("files"));
    store.dispatch(setDockOpen(true));
    const view = renderRail(store);

    await view.user.click(screen.getByRole("button", { name: "Active chats" }));

    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "activity",
    });
  });

  it("forces panels when an unavailable section is selected", async () => {
    const store = createRailStore();
    store.dispatch(
      updateConfig({
        capabilities: {
          filesPanel: false,
          gitPanel: false,
          terminalPanel: false,
        },
      }),
    );
    store.dispatch(setDockOpen(false));
    const view = renderRail(store);

    expect(store.getState().workspace.panelsForced).not.toBe(true);

    await view.user.click(screen.getByRole("button", { name: "Files" }));

    expect(store.getState().workspace.panelsForced).toBe(true);
    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "files",
    });
  });

  it("does not force panels for the always-available agents section", async () => {
    const store = createRailStore();
    store.dispatch(
      updateConfig({
        capabilities: {
          filesPanel: false,
          gitPanel: false,
          terminalPanel: false,
        },
      }),
    );
    store.dispatch(setDockOpen(false));
    const view = renderRail(store);

    await view.user.click(screen.getByRole("button", { name: "Agents" }));

    expect(store.getState().workspace.panelsForced).not.toBe(true);
    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "agents",
    });
  });

  it("toggles the agents dock when the rail button is clicked", async () => {
    const store = createRailStore();
    store.dispatch(setDockSection("files"));
    store.dispatch(setDockOpen(true));
    const view = renderRail(store);

    await view.user.click(screen.getByRole("button", { name: "Agents" }));
    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "agents",
    });

    await view.user.click(screen.getByRole("button", { name: "Agents" }));
    expect(store.getState().workspace.dock?.open).toBe(false);
  });

  it("badges the number of active background agents", async () => {
    const store = createRailStore([agent("agent-1"), agent("agent-2")]);
    renderRail(store);

    const agentsSlot = screen
      .getByRole("button", { name: "Agents" })
      .closest("div");
    if (!agentsSlot) throw new Error("missing agents rail slot");
    await waitFor(() => {
      expect(agentsSlot).toHaveTextContent("2");
    });
  });

  it("omits the agents badge when no agents are running", () => {
    const store = createRailStore([
      agent("agent-done", { status: "completed" }),
    ]);
    renderRail(store);

    const agentsSlot = screen
      .getByRole("button", { name: "Agents" })
      .closest("div");
    if (!agentsSlot) throw new Error("missing agents rail slot");
    expect(agentsSlot).not.toHaveTextContent(/\d/u);
  });

  it("badges unique changed git paths when the git panel is available", async () => {
    server.use(
      http.get("*/v1/git/status", () =>
        HttpResponse.json({
          roots: [
            {
              root: "/repo",
              branch: "main",
              head_detached: false,
              ahead: 0,
              behind: 0,
              staged: [
                {
                  relative_path: "a",
                  absolute_path: "/repo/a",
                  status: "MODIFIED",
                },
              ],
              unstaged: [
                {
                  relative_path: "a",
                  absolute_path: "/repo/a",
                  status: "MODIFIED",
                },
                {
                  relative_path: "b",
                  absolute_path: "/repo/b",
                  status: "DELETED",
                },
              ],
              untracked_included: true,
            },
          ],
        }),
      ),
    );
    const store = createRailStore();
    store.dispatch(setPanelsForced(true));
    renderRail(store);

    const gitSlot = screen.getByRole("button", { name: "Git" }).closest("div");
    if (!gitSlot) throw new Error("missing git rail slot");
    await waitFor(() => {
      expect(gitSlot).toHaveTextContent("2");
    });
  });

  it("badges changes, untracked and conflicted files from the extended status", async () => {
    server.use(
      http.get("*/v1/git/status", () =>
        HttpResponse.json({
          roots: [
            {
              root: "/repo",
              branch: "main",
              head_detached: false,
              ahead: 0,
              behind: 0,
              staged: [],
              unstaged: [],
              untracked_included: true,
              changes: [
                {
                  relative_path: "a",
                  absolute_path: "/repo/a",
                  old_path: null,
                  status: "MODIFIED",
                  staged: true,
                  unstaged: true,
                },
              ],
              untracked: [{ relative_path: "b", absolute_path: "/repo/b" }],
              conflicted: [
                {
                  relative_path: "c",
                  absolute_path: "/repo/c",
                  ours: null,
                  theirs: null,
                  ancestor: null,
                },
              ],
            },
          ],
        }),
      ),
    );
    const store = createRailStore();
    store.dispatch(setPanelsForced(true));
    renderRail(store);

    const gitSlot = screen.getByRole("button", { name: "Git" }).closest("div");
    if (!gitSlot) throw new Error("missing git rail slot");
    await waitFor(() => {
      expect(gitSlot).toHaveTextContent("3");
    });
  });

  it("caps huge git badges while exposing the exact count accessibly and in the tooltip", async () => {
    server.use(
      http.get("*/v1/git/status", () =>
        HttpResponse.json({
          roots: [
            {
              root: "/repo",
              branch: "main",
              head_detached: false,
              ahead: 0,
              behind: 0,
              staged: [],
              unstaged: [],
              untracked_included: true,
              changes: [],
              untracked: Array.from({ length: 10_000 }, (_, index) => ({
                relative_path: `file-${index}`,
                absolute_path: `/repo/file-${index}`,
              })),
            },
          ],
        }),
      ),
    );
    const store = createRailStore();
    store.dispatch(setPanelsForced(true));
    const view = renderRail(store);
    const button = screen.getByRole("button", { name: "Git" });
    await waitFor(() =>
      expect(button).toHaveAccessibleDescription("10000 changed files"),
    );
    const slot = button.closest("div");
    expect(slot?.querySelector(`.${styles.badge}`)).toHaveTextContent("99+");
    await view.user.hover(button);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      "10000 changed files",
    );
  });

  it("keeps visual badges hidden but describes the exact count on the button", async () => {
    const store = createRailStore([agent("agent-1")]);
    renderRail(store);

    const agentsSlot = screen
      .getByRole("button", { name: "Agents" })
      .closest("div");
    if (!agentsSlot) throw new Error("missing agents rail slot");
    await waitFor(() => {
      expect(agentsSlot).toHaveTextContent("1");
    });
    expect(agentsSlot.querySelector('[aria-hidden="true"]')).not.toBeNull();
    expect(screen.getByRole("button", { name: "Agents" })).toHaveAccessibleName(
      "Agents",
    );
    expect(
      screen.getByRole("button", { name: "Agents" }),
    ).toHaveAccessibleDescription("1 active agent");
  });
  it("badges the Active chats slot from the shared engine activity feed", async () => {
    server.use(
      http.get("*/v1/chats/activity", () =>
        HttpResponse.json({
          chats: [
            {
              id: "a",
              title: "Alpha",
              updated_at_ms: Date.now(),
              session_state: "generating",
              recovery_state: null,
              schedule_count: 0,
              paused_schedule_count: 0,
            },
            {
              id: "b",
              title: "Beta",
              updated_at_ms: Date.now(),
              session_state: null,
              recovery_state: "needs_action",
              schedule_count: 1,
              paused_schedule_count: 1,
            },
            {
              id: "c",
              title: "Idle",
              updated_at_ms: 3,
              session_state: "idle",
              recovery_state: null,
              schedule_count: 0,
              paused_schedule_count: 0,
            },
          ],
        }),
      ),
    );
    const store = createRailStore();
    store.dispatch(setPanelsForced(true));
    renderRail(store);

    const button = screen.getByRole("button", { name: "Active chats" });
    await waitFor(() =>
      expect(button).toHaveAccessibleDescription(
        "2 active chats, needs attention",
      ),
    );
    const slot = button.closest("div");
    expect(slot?.querySelector(`.${styles.badge}`)).toHaveTextContent("2");
  });

  it("labels a last-known Active chats badge as stale while offline", async () => {
    server.use(
      http.get("*/v1/chats/activity", () =>
        HttpResponse.json({
          chats: [
            {
              id: "a",
              title: "Alpha",
              updated_at_ms: Date.now(),
              session_state: "generating",
              recovery_state: null,
              schedule_count: 0,
              paused_schedule_count: 0,
            },
          ],
        }),
      ),
    );
    const store = createRailStore();
    store.dispatch(setPanelsForced(true));
    renderRail(store);

    const button = screen.getByRole("button", { name: "Active chats" });
    await waitFor(() =>
      expect(button).toHaveAccessibleDescription("1 active chat"),
    );
    store.dispatch(setBrowserOnline(false));
    await waitFor(() =>
      expect(button).toHaveAccessibleDescription(
        "1 active chat, last known (engine offline)",
      ),
    );
  });

  it("does not poll chat activity when the dock is unavailable", async () => {
    let calls = 0;
    server.use(
      http.get("*/v1/chats/activity", () => {
        calls += 1;
        return HttpResponse.json({ chats: [] });
      }),
    );
    const store = createRailStore();
    store.dispatch(updateConfig({ host: "vscode" }));
    renderRail(store);

    expect(
      screen.getByRole("button", { name: "Active chats" }),
    ).toBeInTheDocument();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(calls).toBe(0);
  });
});
