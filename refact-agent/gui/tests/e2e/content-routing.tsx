import { WorkspaceSurfaceRoutes } from "../../src/features/Workspace/WorkspaceSurfaceRoutes";
import { setTaskActiveChat } from "../../src/features/Tasks/tasksSlice";
import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { useAppSelector } from "../../src/hooks/useAppSelector";
import { useContentRouting } from "../../src/hooks/useContentRouting";
import { updateConfig } from "../../src/features/Config/configSlice";
import { change } from "../../src/features/Pages/pagesSlice";
import { sidebarSectionSnapshotReceived } from "../../src/features/Sidebar/sidebarSlice";
import {
  openTab,
  selectFocusedWorkspaceSurfaceKey,
} from "../../src/features/Workspace/workspaceSlice";
import { Link } from "../../src/components/Link";

const store = setUpStore();
store.dispatch(
  updateConfig({
    host: "web",
    lspPort: 8001,
    capabilities: { filesPanel: true },
  }),
);
store.dispatch(change({ name: "history" }));
store.dispatch(
  sidebarSectionSnapshotReceived({ section: "workspace", status: "ready" }),
);
function Fixture() {
  const routing = useContentRouting(true, true);
  const page = useAppSelector((state) => state.pages.at(-1)?.name);
  const surface = useAppSelector(selectFocusedWorkspaceSurfaceKey);
  const task = useAppSelector((state) =>
    state.tasksUI.openTasks.find((entry) => entry.id === "task-1"),
  );
  const taskView = useAppSelector(
    (state) => state.workspace.contentTaskView?.["task-1"],
  );
  const card = useAppSelector(
    (state) => state.workspace.contentCardByTask?.["task-1"],
  );
  return (
    <>
      <button
        onClick={() => {
          store.dispatch(openTab("file:/workspace/example.ts"));
          store.dispatch(change({ name: "chat" }));
        }}
      >
        Open file from workspace
      </button>
      <button
        onClick={() =>
          store.dispatch(
            setTaskActiveChat({
              taskId: "task-1",
              activeChat: { type: "planner", chatId: "planner-2" },
            }),
          )
        }
      >
        Select second planner
      </button>
      <output data-testid="task-chat">{task?.activeChat?.chatId}</output>
      <output data-testid="task-view">{taskView}</output>
      <output data-testid="task-card">{card}</output>
      <Link href="#/task/task-1">Task</Link>
      <Link href="#/task/task-1/card/T-1">Card</Link>
      <Link href="#/settings/privacy">Privacy</Link>
      <Link href="#/file/%2Fworkspace%2Fexample.ts">File</Link>
      <Link href="refact://chat/linked">Chat</Link>
      <output data-testid="page">{page}</output>
      <output data-testid="surface">{surface}</output>
      <output data-testid="pending">{String(routing.pending)}</output>
      <output data-testid="error">{routing.error}</output>
    </>
  );
}
createRoot(document.getElementById("root")!).render(
  <Provider store={store}>
    <WorkspaceSurfaceRoutes>
      <Fixture />
    </WorkspaceSurfaceRoutes>
  </Provider>,
);
