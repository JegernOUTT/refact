import type { PaneNode } from "../ChatPanes/panesTree";
import type { AppWorkspaceLayout } from "./appWorkspace";

/** Compare portable meaning, not default spelling or floating point round-off. */
export function canonicalAppLayout(
  layout: AppWorkspaceLayout,
): AppWorkspaceLayout {
  if (layout.kind === "leaf")
    return { kind: "leaf", tabs: layout.tabs, active: layout.active ?? 0 };
  const total = layout.sizes.reduce((sum, size) => sum + size, 0);
  return {
    kind: "split",
    direction: layout.direction,
    sizes: layout.sizes.map((size) => Math.round((size / total) * 1e12) / 1e12),
    children: layout.children.map(canonicalAppLayout),
  };
}

/** Project only portable surfaces; never mutate or discard the live file tabs. */
export function paneToAppLayout(
  node: PaneNode,
  chatId: string,
  bindingIds?: ReadonlySet<string>,
): AppWorkspaceLayout | null {
  if (node.kind === "split") {
    const kept = node.children.flatMap((child, index) => {
      const layout = paneToAppLayout(child, chatId, bindingIds);
      return layout ? [{ layout, size: node.sizes[index] ?? 1 }] : [];
    });
    if (!kept.length) return null;
    if (kept.length === 1) return kept[0].layout;
    const total = kept.reduce((sum, child) => sum + child.size, 0);
    return {
      kind: "split",
      direction: node.dir === "row" ? "horizontal" : "vertical",
      sizes: kept.map((child) => child.size / total),
      children: kept.map((child) => child.layout),
    };
  }
  const keys = node.tabIds.filter(
    (key) =>
      key === `chat:${chatId}` ||
      (key.startsWith("app:") &&
        !key.startsWith("app:standalone:") &&
        (!bindingIds || bindingIds.has(key.slice(4)))),
  );
  if (!keys.length) return null;
  return {
    kind: "leaf",
    tabs: keys.map((key) =>
      key === `chat:${chatId}`
        ? { kind: "chat", ref: "self" }
        : { kind: "app", binding_id: key.slice(4) },
    ),
    active: Math.max(0, keys.indexOf(node.activeTabId ?? "")),
  };
}

export function appLayoutToPane(
  layout: AppWorkspaceLayout,
  chatId: string,
  path = `app-root:${chatId}`,
): PaneNode {
  if (layout.kind === "leaf") {
    const tabIds = layout.tabs.map((tab) =>
      tab.kind === "chat" ? `chat:${chatId}` : `app:${tab.binding_id}`,
    );
    return {
      kind: "leaf",
      id: path,
      tabIds,
      activeTabId: tabIds[layout.active ?? 0] ?? null,
    };
  }
  return {
    kind: "split",
    id: path,
    dir: layout.direction === "horizontal" ? "row" : "col",
    sizes: layout.sizes,
    children: layout.children.map((child, index) =>
      appLayoutToPane(child, chatId, `${path}:${index}`),
    ),
  };
}
