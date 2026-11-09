import { useAppSelector } from "../../hooks";
import {
  selectAllThreads,
  selectTabsDisplayData,
  type TabDisplayData,
} from "../Chat/Thread";
import { selectOpenTasksFromRoot, type OpenTask } from "../Tasks/tasksSlice";
import { getChatActivity, type ChatActivity } from "../../utils/chatActivity";
import {
  worktreeDiffDescriptorFromSurfaceKey,
  gitDiffDescriptorFromSurfaceKey,
  isMainSurfaceKind,
  parseSurfaceKey,
  type MainSurfaceKind,
  type SurfaceKey,
} from "./surfaceKey";
import {
  gitDiffTitle,
  gitDiffTooltip,
  joinRepoPath,
} from "./GitPanel/diff/gitDiffDescriptor";

type TabSurfaceKind =
  | "app"
  | "chat"
  | "task"
  | "buddy"
  | "dashboard"
  | "design"
  | "file"
  | "worktreediff"
  | "gitdiff"
  | MainSurfaceKind;

const MAIN_SURFACE_INFO = { git: { title: "Git" } } as const;

const pathBasename = (path: string): string => {
  const normalized = path.replace(/\\/g, "/");
  return normalized.slice(normalized.lastIndexOf("/") + 1) || path;
};

export type TabDisplayInfo = {
  title: string;
  tooltip?: string;
  kind: TabSurfaceKind;
  mode?: string;
  filePath?: string;
  project?: string;
  root?: string;
  branch?: string;
  taskId?: string;
  context?: string;
  session_state?: string;
  activity?: ChatActivity;
  is_buddy_chat?: boolean;
  is_task_chat?: boolean;
  unreadNotificationCount: number;
};

function fallbackSurfaceTitle(surfaceKey: SurfaceKey): string {
  try {
    const parsed = parseSurfaceKey(surfaceKey);
    if (parsed.kind === "dashboard") return "Dashboard";
    if (parsed.kind === "buddy") return "Buddy";
    if (parsed.kind === "gitdiff") return "Diff";
    const prefix = parsed.kind[0].toUpperCase();
    return `${prefix}${parsed.kind.slice(1)} ${parsed.id}`;
  } catch {
    return surfaceKey;
  }
}

function taskSessionState(task: OpenTask | undefined): string | undefined {
  const activeChat = task?.activeChat;
  if (activeChat?.type === "planner") {
    return task?.plannerChats.find(
      (planner) => planner.id === activeChat.chatId,
    )?.sessionState;
  }
  return task?.plannerChats[0]?.sessionState;
}

function threadContext(
  runtime: ReturnType<typeof selectAllThreads>[string],
): Partial<TabDisplayInfo> {
  const thread = runtime?.thread;
  return {
    mode: thread?.mode,
    project: thread?.project_name ?? thread?.integration?.project,
    root: thread?.worktree?.root,
    branch: thread?.worktree?.branch ?? undefined,
    taskId:
      thread?.task_meta?.task_id ?? thread?.worktree?.task_id ?? undefined,
  };
}

export function displayInfoForSurface(
  surfaceKey: SurfaceKey,
  tabsById: ReadonlyMap<string, TabDisplayData>,
  threads: ReturnType<typeof selectAllThreads>,
  tasksById: ReadonlyMap<string, OpenTask>,
): TabDisplayInfo {
  let parsed: ReturnType<typeof parseSurfaceKey>;
  try {
    parsed = parseSurfaceKey(surfaceKey);
  } catch {
    return {
      title: fallbackSurfaceTitle(surfaceKey),
      kind: "dashboard",
      unreadNotificationCount: 0,
    };
  }

  if (parsed.kind === "app")
    return { title: "App", kind: "app", unreadNotificationCount: 0 };

  if (parsed.kind === "task") {
    const task = tasksById.get(parsed.id);
    const planner =
      task?.plannerChats.find((item) => item.id === task.activeChat?.chatId) ??
      task?.plannerChats[0];
    const runtime = task?.activeChat
      ? threads[task.activeChat.chatId]
      : undefined;
    return {
      ...threadContext(runtime),
      mode: runtime?.thread.mode ?? planner?.mode,
      taskId: task?.id,
      activity: runtime ? getChatActivity(runtime) : undefined,
      title: task?.name ?? fallbackSurfaceTitle(surfaceKey),
      kind: "task",
      session_state: runtime?.session_state ?? taskSessionState(task),
      unreadNotificationCount: task?.activeChat
        ? tabsById.get(task.activeChat.chatId)?.unreadNotificationCount ?? 0
        : 0,
    };
  }

  if (parsed.kind === "buddy") {
    return {
      title: "Buddy",
      kind: "buddy",
      unreadNotificationCount: 0,
    };
  }

  if (parsed.kind === "dashboard") {
    return {
      title: "Dashboard",
      kind: "dashboard",
      unreadNotificationCount: 0,
    };
  }

  if (parsed.kind === "design") {
    return {
      title: parsed.id === "main" ? "Design" : `Design ${parsed.id}`,
      kind: "design",
      unreadNotificationCount: 0,
    };
  }

  if (parsed.kind === "file") {
    return {
      title: pathBasename(parsed.id),
      tooltip: parsed.id,
      filePath: parsed.id,
      kind: "file",
      unreadNotificationCount: 0,
    };
  }

  if (parsed.kind === "worktreediff") {
    const descriptor = worktreeDiffDescriptorFromSurfaceKey(surfaceKey);
    return {
      title: `Diff · ${descriptor?.label ?? "Worktree"}`,
      tooltip: descriptor?.root ?? descriptor?.worktreeId,
      root: descriptor?.root,
      kind: "worktreediff",
      unreadNotificationCount: 0,
    };
  }

  if (parsed.kind === "gitdiff") {
    const descriptor = gitDiffDescriptorFromSurfaceKey(surfaceKey);
    return {
      title: descriptor ? gitDiffTitle(descriptor) : "Diff",
      tooltip: descriptor ? gitDiffTooltip(descriptor) : undefined,
      filePath: descriptor
        ? joinRepoPath(descriptor.root, descriptor.path)
        : undefined,
      root: descriptor?.root,
      context: descriptor ? gitDiffTooltip(descriptor) : undefined,
      kind: "gitdiff",
      unreadNotificationCount: 0,
    };
  }

  if (isMainSurfaceKind(parsed.kind)) {
    return {
      title: MAIN_SURFACE_INFO[parsed.kind].title,
      kind: parsed.kind,
      unreadNotificationCount: 0,
    };
  }

  const chatId = surfaceKey.slice("chat:".length);
  const tab = tabsById.get(chatId);
  if (tab) {
    return {
      ...tab,
      ...threadContext(threads[chatId]),
      mode: threads[chatId]?.thread.mode ?? tab.mode,
      tooltip: tab.title,
      kind: "chat",
      activity: threads[chatId]
        ? getChatActivity(threads[chatId])
        : tab.activity,
    };
  }

  const runtime = threads[chatId];
  return {
    ...threadContext(runtime),
    title: runtime?.thread.title ?? fallbackSurfaceTitle(surfaceKey),
    kind: "chat",
    session_state: runtime?.session_state,
    activity: getChatActivity(runtime),
    is_buddy_chat: Boolean(runtime?.thread.buddy_meta?.is_buddy_chat),
    is_task_chat: Boolean(runtime?.thread.is_task_chat),
    unreadNotificationCount: 0,
  };
}

export type TabDisplayInput = TabDisplayInfo;

export function useTabDisplay() {
  const views = useAppSelector((state) => state.apps.views);
  const tabs = useAppSelector(selectTabsDisplayData);
  const threads = useAppSelector(selectAllThreads);
  const tasks = useAppSelector(selectOpenTasksFromRoot);
  const tabsById = new Map(tabs.map((tab) => [tab.id, tab]));
  const tasksById = new Map(tasks.map((task) => [task.id, task]));
  return (key: SurfaceKey) =>
    views[key]
      ? {
          title: views[key].name,
          kind: "app" as const,
          unreadNotificationCount: 0,
        }
      : displayInfoForSurface(key, tabsById, threads, tasksById);
}
