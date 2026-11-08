import { describe, expect, it } from "vitest";
import { setUpStore } from "../app/store";
import { change } from "../features/Pages/pagesSlice";
import {
  openTab,
  setContentCard,
  setContentTaskView,
} from "../features/Workspace/workspaceSlice";
import { openTask, setTaskActiveChat } from "../features/Tasks/tasksSlice";
import {
  buildContentRoute,
  contentRouteHref,
  parseContentRoute,
  routeFromContentLink,
  selectContentRoute,
  type ContentRoute,
} from "./contentRoutes";

describe("content routes", () => {
  it.each<ContentRoute>([
    { type: "chat", id: "chat-1" },
    { type: "task", id: "task_1", cardId: "T-2" },
    { type: "file", path: "/workspace/my file #1?.ts" },
    { type: "file", path: "C:\\work\\日本語.ts" },
    { type: "page", path: "settings/privacy" },
    { type: "app", id: "missing-app" },
  ])("round trips %j without a server path", (route) => {
    expect(parseContentRoute(buildContentRoute(route))).toEqual(route);
    expect(contentRouteHref(route, "/index.html")).toBe(
      `/index.html${buildContentRoute(route)}`,
    );
  });
  it.each([
    "#/chat/",
    "#/chat/a/b",
    "#/task/a/chat/b",
    "#/chat/%2f",
    "#/file/%00",
    "#/file/%zz",
    "#/constructor",
    "#/__proto__",
    "#/settings/unknown",
    "javascript:alert(1)",
  ])("rejects malformed route %s", (hash) => {
    expect(parseContentRoute(hash)).toBeNull();
  });
  it("translates legacy links without copying bootstrap query secrets", () => {
    window.history.replaceState(null, "", "/?token=secret&apiKey=private");
    const route = routeFromContentLink("refact://task/task-1/card/T-1");
    expect(route).toEqual({ type: "task", id: "task-1", cardId: "T-1" });
    expect(route && contentRouteHref(route)).toBe("/#/task/task-1/card/T-1");
    expect(routeFromContentLink("https://evil.example/#/chat/a")).toBeNull();
    expect(routeFromContentLink("/?token=secret#/chat/a")).toBeNull();
  });
  it("uses focused file and task surfaces, never their context chat", () => {
    const store = setUpStore();
    store.dispatch(change({ name: "chat" }));
    store.dispatch(openTab("file:/workspace/test.ts"));
    expect(selectContentRoute(store.getState())).toEqual({
      type: "file",
      path: "/workspace/test.ts",
    });
    store.dispatch(openTab("task:task-1"));
    store.dispatch(setContentCard({ taskId: "task-1", cardId: "T-2" }));
    expect(selectContentRoute(store.getState())).toEqual({
      type: "task",
      id: "task-1",
      cardId: "T-2",
    });
  });
  it("does not infer Chat from an active planner before the task view resolves", () => {
    const store = setUpStore();
    store.dispatch(change({ name: "chat" }));
    store.dispatch(openTab("task:task-1"));
    store.dispatch(openTask({ id: "task-1", name: "Task" }));
    store.dispatch(
      setTaskActiveChat({
        taskId: "task-1",
        activeChat: { type: "planner", chatId: "planner-1" },
      }),
    );
    expect(selectContentRoute(store.getState())).toEqual({
      type: "task",
      id: "task-1",
    });
    store.dispatch(setContentTaskView({ taskId: "task-1", view: "chat" }));
    expect(selectContentRoute(store.getState())).toEqual({
      type: "chat",
      id: "planner-1",
    });
  });
});
