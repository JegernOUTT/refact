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
import { createChatWithId } from "../../Chat/Thread";
import {
  openTab,
  setActiveTab,
  setDockOpen,
  setDockSection,
  setPanelsForced,
} from "../workspaceSlice";
import { makeSurfaceKey } from "../surfaceKey";
import { ActivityRail } from "./ActivityRail";

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

  it("renders one rail button per workspace section", () => {
    renderRail(createRailStore());

    const rail = screen.getByRole("navigation", { name: "Workspace sections" });
    expect(rail).toBeInTheDocument();
    for (const label of ["Files", "Git", "Agents", "Tasks"]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
  });

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

    for (const label of ["Files", "Git", "Agents", "Tasks"]) {
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

    await view.user.click(screen.getByRole("button", { name: "Tasks" }));

    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "tasks",
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

  it("records agents open and close intent for the focused chat", async () => {
    const store = createRailStore();
    store.dispatch(setDockSection("files"));
    store.dispatch(setDockOpen(true));
    const view = renderRail(store);

    await view.user.click(screen.getByRole("button", { name: "Agents" }));
    expect(store.getState().agentsPanel.userClosedByChat[chatId]).toBe(false);

    await view.user.click(screen.getByRole("button", { name: "Agents" }));
    expect(store.getState().agentsPanel.userClosedByChat[chatId]).toBe(true);
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

  it("keeps rail badges out of the accessibility tree", async () => {
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
  });
});
