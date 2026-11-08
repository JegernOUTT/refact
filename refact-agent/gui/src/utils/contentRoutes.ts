import { parseRefactLink } from "../contexts/internalLinkUtils";
import type { RootState } from "../app/store";
import type { Page } from "../features/Pages/pagesSlice";
import { selectFocusedWorkspaceSurfaceKey } from "../features/Workspace/workspaceSlice";

export type ContentRoute =
  | { type: "chat"; id: string }
  | { type: "task"; id: string; cardId?: string }
  | { type: "app"; id: string }
  | { type: "file"; path: string }
  | { type: "page"; path: string };

const pages: Record<string, Page> = {
  dashboard: { name: "history" },
  tasks: { name: "tasks list" },
  knowledge: { name: "knowledge graph" },
  "code-intel": { name: "code intel" },
  statistics: { name: "stats dashboard" },
  performance: { name: "performance" },
  daemon: { name: "refact daemon" },
  "bug-report": { name: "bug report" },
  "settings/general": { name: "general settings" },
  "settings/privacy": { name: "privacy settings" },
  "settings/shell": { name: "shell settings" },
  "settings/indexing": { name: "indexing settings" },
  "settings/browser": { name: "browser settings" },
  "settings/limits": { name: "limits settings" },
  "settings/providers": { name: "providers page" },
  "settings/models": { name: "default models" },
  "settings/integrations": { name: "integrations page" },
  "settings/customization": { name: "customization" },
  extensions: { name: "extensions" },
  marketplace: { name: "marketplace hub" },
  scheduler: { name: "scheduler" },
  buddy: { name: "buddy" },
};
export const contentRoutePage = (path: string): Page | undefined =>
  Object.prototype.hasOwnProperty.call(pages, path) ? pages[path] : undefined;
const validId = (id: string) => /^[A-Za-z0-9_-]{1,128}$/u.test(id);
const validPath = (path: string) =>
  path.length > 0 &&
  path.length <= 8192 &&
  ![...path].some(
    (char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127,
  );

export function parseContentRoute(hash: string): ContentRoute | null {
  if (!hash.startsWith("#/")) return null;
  const path = hash.slice(2);
  if (Object.prototype.hasOwnProperty.call(pages, path))
    return { type: "page", path };
  const parts = path.split("/");
  try {
    if (parts[0] === "file" && parts.length === 2) {
      const filePath = decodeURIComponent(parts[1]);
      return validPath(filePath) ? { type: "file", path: filePath } : null;
    }
    const id = decodeURIComponent(parts[1] ?? "");
    if (!validId(id)) return null;
    if ((parts[0] === "chat" || parts[0] === "app") && parts.length === 2)
      return { type: parts[0], id };
    if (parts[0] === "task") {
      if (parts.length === 2) return { type: "task", id };
      const cardId = decodeURIComponent(parts[3] ?? "");
      if (parts.length === 4 && parts[2] === "card" && validId(cardId))
        return { type: "task", id, cardId };
    }
  } catch {
    return null;
  }
  return null;
}

export function buildContentRoute(route: ContentRoute): string {
  let hash: string;
  if (route.type === "page") hash = `#/${route.path}`;
  else if (route.type === "file")
    hash = `#/file/${encodeURIComponent(route.path)}`;
  else
    hash = `#/${route.type}/${encodeURIComponent(route.id)}${
      route.type === "task" && route.cardId
        ? `/card/${encodeURIComponent(route.cardId)}`
        : ""
    }`;
  return parseContentRoute(hash) ? hash : "";
}

export function contentRouteHref(
  route: ContentRoute,
  pathname = window.location.pathname,
): string {
  const hash = buildContentRoute(route);
  return hash ? `${pathname}${hash}` : "";
}

export function routeFromContentLink(href: string): ContentRoute | null {
  if (href.startsWith("refact://")) {
    const target = parseRefactLink(href);
    return target && target.type !== "worktree" ? target : null;
  }
  if (href.startsWith("#/")) return parseContentRoute(href);
  try {
    const url = new URL(href, window.location.href);
    if (
      url.origin !== window.location.origin ||
      url.pathname !== window.location.pathname ||
      url.search
    )
      return null;
    return parseContentRoute(url.hash);
  } catch {
    return null;
  }
}

export function selectContentRoute(state: RootState): ContentRoute | null {
  const page = state.pages.at(-1);
  if (!page || page.name === "login page") return null;
  if (page.name === "task workspace")
    return { type: "task", id: page.taskId, cardId: page.cardId };
  if (page.name === "chat") {
    const surface = selectFocusedWorkspaceSurfaceKey(state);
    if (surface?.startsWith("chat:"))
      return { type: "chat", id: surface.slice(5) };
    if (surface?.startsWith("task:")) {
      const taskId = surface.slice(5);
      const cardId = state.workspace.contentCardByTask?.[taskId] ?? undefined;
      const activeChat = state.tasksUI.openTasks.find(
        (task) => task.id === taskId,
      )?.activeChat;
      if (
        !cardId &&
        state.workspace.contentTaskView?.[taskId] === "chat" &&
        activeChat
      )
        return { type: "chat", id: activeChat.chatId };
      return { type: "task", id: taskId, cardId };
    }
    if (surface?.startsWith("file:"))
      return { type: "file", path: surface.slice(5) };
    if (surface === "buddy:home") return { type: "page", path: "buddy" };
    return null;
  }
  const path = Object.keys(pages).find((key) => pages[key].name === page.name);
  return path ? { type: "page", path } : null;
}

export const isPrimaryContentClick = (event: {
  button: number;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  defaultPrevented: boolean;
}) =>
  !event.defaultPrevented &&
  event.button === 0 &&
  !event.metaKey &&
  !event.ctrlKey &&
  !event.shiftKey &&
  !event.altKey;

export function contentRouteForSurface(surface: string): ContentRoute | null {
  if (surface.startsWith("chat:"))
    return { type: "chat", id: surface.slice(5) };
  if (surface.startsWith("task:"))
    return { type: "task", id: surface.slice(5) };
  if (surface.startsWith("file:"))
    return { type: "file", path: surface.slice(5) };
  if (surface === "buddy:home") return { type: "page", path: "buddy" };
  return null;
}
