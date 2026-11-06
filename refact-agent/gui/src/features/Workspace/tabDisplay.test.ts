import { describe, expect, it } from "vitest";
import type { TabDisplayData } from "../Chat/Thread";
import type { OpenTask } from "../Tasks/tasksSlice";
import { makeGitDiffSurfaceKey } from "./surfaceKey";
import { displayInfoForSurface } from "./tabDisplay";

describe("tab display details", () => {
  it("preserves genuine historical mode, badge and notification metadata", () => {
    const tab: TabDisplayData = {
      id: "history",
      title: "Full history title",
      mode: "task_planner",
      activity: "waiting_user",
      session_state: "waiting_user_input",
      is_buddy_chat: true,
      is_task_chat: true,
      unreadNotificationCount: 17,
    };
    expect(
      displayInfoForSurface(
        "chat:history",
        new Map([[tab.id, tab]]),
        {},
        new Map(),
      ),
    ).toMatchObject({
      ...tab,
      kind: "chat",
      tooltip: tab.title,
    });
  });

  it("retains full Windows file paths separately from their visible basename", () => {
    expect(
      displayInfoForSurface(
        "file:C:\\project\\long directory\\file.ts",
        new Map(),
        {},
        new Map(),
      ),
    ).toMatchObject({
      title: "file.ts",
      filePath: "C:\\project\\long directory\\file.ts",
      kind: "file",
    });
  });

  it("exposes existing planner mode and task identity without invented branch or project", () => {
    const task: OpenTask = {
      id: "task",
      name: "Full task title",
      activeChat: { type: "planner", chatId: "planner" },
      plannerChats: [
        {
          id: "planner",
          title: "Planner",
          createdAt: "",
          updatedAt: "",
          mode: "task_planner",
          sessionState: "generating",
        },
      ],
    };
    const display = displayInfoForSurface(
      "task:task",
      new Map([
        [
          "planner",
          {
            id: "planner",
            title: "Planner",
            mode: "task_planner",
            activity: "working",
            unreadNotificationCount: 3,
          },
        ],
      ]),
      {},
      new Map([[task.id, task]]),
    );
    expect(display).toMatchObject({
      title: task.name,
      kind: "task",
      taskId: "task",
      mode: "task_planner",
      session_state: "generating",
      unreadNotificationCount: 3,
    });
    expect(display.branch).toBeUndefined();
    expect(display.project).toBeUndefined();
  });

  it("exposes diff root, full file path and existing revision context", () => {
    const key = makeGitDiffSurfaceKey({
      root: "/repo",
      path: "src/file.ts",
      old_path: null,
      left: { rev: "HEAD", path: "src/file.ts" },
      right: { rev: "WORKTREE", path: "src/file.ts" },
      kind: "working",
    });
    expect(displayInfoForSurface(key, new Map(), {}, new Map())).toMatchObject({
      kind: "gitdiff",
      title: "file.ts",
      root: "/repo",
      filePath: "/repo/src/file.ts",
      context: "src/file.ts (HEAD ↔ Working tree)",
    });
  });
});
