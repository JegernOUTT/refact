import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import { setUpStore } from "../../app/store";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { createChatWithId, switchToThread } from "../Chat/Thread";
import { useThreadId } from "../Chat/Thread/ChatThreadContext";
import { openTask, setTaskActiveChat } from "../Tasks/tasksSlice";
import { tasksApi } from "../../services/refact/tasks";
import {
  saveTaskWorkspaceTab,
  setProjectStorageNamespace,
} from "../../utils/chatUiPersistence";
import { GroupSplitView } from "./GroupSplitView";
import {
  focusPane,
  setPaneActive,
  selectFocusedWorkspaceChatId,
} from "./workspaceSlice";

vi.hoisted(() => {
  vi.resetModules();
});

vi.mock("../Chat/Chat", () => ({
  Chat: function ChatProbe() {
    const chatId = useThreadId();
    return <div data-testid={`chat-probe-${chatId}`}>{chatId}</div>;
  },
}));
vi.mock("../Chat", async () => ({ Chat: (await import("../Chat/Chat")).Chat }));
vi.mock("../Dashboard/hooks/useDashboardLayout", () => ({
  useDashboardLayout: () => "wide",
}));

describe("mixed Task and Chat pane focus convergence", () => {
  it("does not borrow a page Task chat when the focused Task has no open UI", () => {
    const store = setUpStore();
    const state = store.getState();
    expect(
      selectFocusedWorkspaceChatId({
        ...state,
        pages: [{ name: "task workspace", taskId: "page-task" }],
        tasksUI: {
          openTasks: [
            {
              id: "page-task",
              plannerChats: [],
              activeChat: { chatId: "page-chat" },
            },
          ],
        },
        workspace: {
          ...state.workspace,
          tabs: ["task:missing"],
          activeTabId: "task:missing",
          groups: {
            "task:missing": {
              focusedLeafId: "missing",
              root: {
                kind: "leaf",
                id: "missing",
                tabIds: ["task:missing"],
                activeTabId: "task:missing",
              },
            },
          },
        },
      }),
    ).toBeNull();
  });
  it("lets only the focused active Task synchronize while retaining two real Task workspaces", async () => {
    localStorage.clear();
    sessionStorage.clear();
    setProjectStorageNamespace(undefined);
    for (const id of ["a", "b"]) saveTaskWorkspaceTab(id, "chat");
    server.use(
      http.get("*/v1/tasks/:id", ({ params }) =>
        HttpResponse.json({
          meta: {
            id: String(params.id),
            name: `Task ${String(params.id)}`,
            status: "active",
            created_at: "t0",
            updated_at: "t0",
            cards_total: 1,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 1,
            base_branch: "main",
          },
        }),
      ),
      http.get("*/v1/tasks/:id/board", ({ params }) =>
        HttpResponse.json({
          schema_version: 1,
          rev: 1,
          columns: [{ id: "doing", title: "Doing" }],
          cards: [
            {
              id: `card-${String(params.id)}`,
              title: `Card ${String(params.id)}`,
              column: "doing",
              priority: "P1",
              depends_on: [],
              instructions: "",
              assignee: "agent",
              agent_chat_id: `agent-${String(params.id)}`,
              status_updates: [],
              final_report: null,
              created_at: "t0",
              started_at: "t0",
              completed_at: null,
              target_files: [],
            },
          ],
        }),
      ),
      http.get("*/v1/tasks/:id/trajectories/:role", () =>
        HttpResponse.json([]),
      ),
      http.get("*/v1/trajectories/:id", ({ params }) =>
        HttpResponse.json({
          id: String(params.id),
          title: "Stored agent",
          model: "",
          mode: "TASK_AGENT",
          messages: [],
          created_at: "t0",
          updated_at: "t0",
        }),
      ),
      http.get("*/v1/worktrees", () => HttpResponse.json({ worktrees: [] })),
      http.get("*/v1/chat-modes", () =>
        HttpResponse.json({ modes: [], errors: [] }),
      ),
    );
    const initialStore = setUpStore({
      config: { host: "web", lspPort: 8001, apiKey: null, themeProps: {} },
    });
    initialStore.dispatch(
      createChatWithId({ id: "regular", title: "Regular chat" }),
    );
    initialStore.dispatch(
      createChatWithId({ id: "hidden-control", title: "Hidden control" }),
    );
    for (const id of ["a", "b"]) {
      initialStore.dispatch(openTask({ id, name: `Task ${id}` }));
      initialStore.dispatch(
        setTaskActiveChat({
          taskId: id,
          activeChat: {
            type: "agent",
            cardId: `card-${id}`,
            chatId: `agent-${id}`,
          },
        }),
      );
    }
    const initial = initialStore.getState();
    const store = setUpStore({
      ...initial,
      workspace: {
        ...initial.workspace,
        tabs: ["task:a"],
        activeTabId: "task:a",
        groups: {
          "task:a": {
            focusedLeafId: "a",
            root: {
              kind: "split",
              id: "split",
              dir: "row",
              sizes: [1 / 3, 1 / 3, 1 / 3],
              children: [
                {
                  kind: "leaf",
                  id: "a",
                  tabIds: ["task:a"],
                  activeTabId: "task:a",
                },
                {
                  kind: "leaf",
                  id: "b",
                  tabIds: ["task:b", "chat:hidden-control"],
                  activeTabId: "task:b",
                },
                {
                  kind: "leaf",
                  id: "chat",
                  tabIds: ["chat:regular"],
                  activeTabId: "chat:regular",
                },
              ],
            },
          },
        },
      },
    });
    const tabId = store.getState().workspace.activeTabId;
    if (!tabId) throw new Error("Missing mixed workspace root");
    const dispatch = vi.spyOn(store, "dispatch");
    const switches = () =>
      dispatch.mock.calls.filter(
        ([action]) =>
          typeof action === "object" &&
          "type" in action &&
          action.type === switchToThread.type,
      );
    const { unmount } = render(<GroupSplitView tabId={tabId} />, { store });
    async function settles(id: string) {
      await waitFor(() =>
        expect(store.getState().chat.current_thread_id).toBe(id),
      );
      const count = switches().length;
      for (let turn = 0; turn < 3; turn++) {
        await act(async () => {
          await new Promise((resolve) => setTimeout(resolve, 20));
        });
        expect(store.getState().chat.current_thread_id).toBe(id);
        expect(switches()).toHaveLength(count);
      }
      expect(count).toBeLessThanOrEqual(8);
    }
    await waitFor(() => {
      expect(store.getState().chat.threads["agent-a"]).toBeDefined();
      expect(store.getState().chat.threads["agent-b"]).toBeDefined();
    });
    await settles("agent-a");
    const taskA = screen.getByLabelText("Workspace pane a");
    const taskB = screen.getByLabelText("Workspace pane b");
    fireEvent.mouseDown(within(taskA).getByRole("tab", { name: "Chat" }));
    fireEvent.click(within(taskA).getByRole("tab", { name: "Chat" }));
    fireEvent.mouseDown(within(taskB).getByRole("tab", { name: "Chat" }));
    fireEvent.click(within(taskB).getByRole("tab", { name: "Chat" }));
    expect(within(taskA).getByTestId("chat-probe-agent-a")).toBeVisible();
    expect(within(taskB).getByTestId("chat-probe-agent-b")).toBeVisible();
    act(() => {
      store.dispatch(focusPane({ tabId, leafId: "b" }));
    });
    await settles("agent-b");
    act(() => {
      store.dispatch(focusPane({ tabId, leafId: "chat" }));
    });
    await settles("regular");
    expect(within(taskA).getByTestId("chat-probe-agent-a")).toBeVisible();
    expect(within(taskB).getByTestId("chat-probe-agent-b")).toBeVisible();
    act(() => {
      store.dispatch(
        setPaneActive({
          tabId,
          leafId: "b",
          surfaceKey: "chat:hidden-control",
        }),
      );
    });
    await settles("hidden-control");
    expect(within(taskB).getByTestId("chat-probe-agent-b")).not.toBeVisible();
    expect(
      within(taskB).getByTestId("chat-probe-hidden-control"),
    ).toBeVisible();
    act(() => {
      store.dispatch(
        setPaneActive({ tabId, leafId: "b", surfaceKey: "task:b" }),
      );
    });
    await settles("agent-b");
    const before = switches().length;
    act(() => {
      store.dispatch(switchToThread({ id: "regular", openTab: false }));
    });
    await settles("agent-b");
    expect(switches().length - before).toBe(2);
    expect(
      store.getState().tasksUI.openTasks.map((task) => task.activeChat?.chatId),
    ).toEqual(["agent-a", "agent-b"]);
    unmount();
    dispatch.mockRestore();
    act(() => {
      store.dispatch(tasksApi.util.resetApiState());
    });
  });
});
