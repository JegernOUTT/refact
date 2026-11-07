import { type CSSProperties } from "react";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { findLeaf, type LeafPane, type PaneNode } from "../ChatPanes/panesTree";
import { push, selectCurrentPage } from "../Pages/pagesSlice";
import {
  PaneTabStrip,
  type PaneNavigationEntry,
  type PaneTabOwner,
} from "./PaneTabStrip";
import { isSplittableSurface, type SurfaceKey } from "./surfaceKey";
import {
  closeTab,
  closePaneTab,
  selectActiveTabId,
  selectTabs,
  selectWorkspaceGroups,
  setActiveTab,
} from "./workspaceSlice";
import { usePaneTabGeometry } from "./usePaneTabGeometry";
import type { PaneTabFitChange } from "./paneTabFit";
import styles from "./WorkspacePaneTabs.module.css";

export function WorkspaceToolbarTabs({
  onFitChange,
}: {
  onFitChange?: PaneTabFitChange;
}) {
  const dispatch = useAppDispatch();
  const tabs = useAppSelector(selectTabs);
  const groups = useAppSelector(selectWorkspaceGroups);
  const activeTabId = useAppSelector(selectActiveTabId);
  const page = useAppSelector(selectCurrentPage);
  const ownerBySurface: Record<SurfaceKey, PaneTabOwner> = {};
  const surfaceKeys: SurfaceKey[] = [];
  const visit = (node: PaneNode, tabId: SurfaceKey) => {
    if (node.kind === "split") {
      node.children.forEach((child) => visit(child, tabId));
      return;
    }
    for (const key of node.tabIds) {
      if (key in ownerBySurface) continue;
      ownerBySurface[key] = { tabId, leaf: node };
      surfaceKeys.push(key);
    }
  };
  for (const tabId of Array.from(new Set([...tabs, ...Object.keys(groups)]))) {
    const group = groups[tabId];
    if (group) visit(group.root, tabId);
  }
  for (const key of tabs) {
    if (isSplittableSurface(key) && !surfaceKeys.includes(key))
      surfaceKeys.push(key);
  }
  const activeGroup = activeTabId ? groups[activeTabId] : undefined;
  const leaves: LeafPane[] = [];
  const collectLeaves = (node: PaneNode) => {
    if (node.kind === "leaf") leaves.push(node);
    else node.children.forEach(collectLeaves);
  };
  if (activeGroup) collectLeaves(activeGroup.root);
  const { slotRef, sections } = usePaneTabGeometry(
    page?.name === "chat" && activeGroup ? activeTabId : null,
    JSON.stringify(leaves.map((pane) => pane.id)),
  );
  const focusedLeaf = activeGroup
    ? findLeaf(activeGroup.root, activeGroup.focusedLeafId)
    : null;
  const leaf: LeafPane = {
    kind: "leaf",
    id: "toolbar",
    tabIds: surfaceKeys,
    activeTabId:
      focusedLeaf?.activeTabId ??
      focusedLeaf?.tabIds[0] ??
      (activeTabId && isSplittableSurface(activeTabId) ? activeTabId : null),
  };
  const activate = (key: SurfaceKey) => {
    dispatch(setActiveTab(key));
    if (page?.name !== "chat") dispatch(push({ name: "chat" }));
  };
  const entries: PaneNavigationEntry[] = [];
  const selected = page && page.name !== "chat" ? null : undefined;
  const strip = (
    <PaneTabStrip
      onFitChange={onFitChange}
      leaf={leaf}
      tabId={activeTabId ?? "workspace-navigation"}
      navigationEntries={entries}
      selectedNavigationKey={selected}
      onActivateLeaf={activate}
      hidePaneActions
      ownerBySurface={ownerBySurface}
      tablistLabel="Workspace tabs"
    />
  );
  const foreignEntries: PaneNavigationEntry[] = surfaceKeys
    .filter((key) => !leaves.some((pane) => pane.tabIds.includes(key)))
    .map((key) => ({
      key,
      surfaceKey: key,
      onActivate: () => activate(key),
      onClose: () => {
        const owner = ownerBySurface[key];
        if (key in ownerBySurface)
          dispatch(
            closePaneTab({
              tabId: owner.tabId,
              leafId: owner.leaf.id,
              surfaceKey: key,
            }),
          );
        else dispatch(closeTab(key));
      },
    }));
  return (
    <div
      ref={slotRef}
      className={styles.band}
      role="group"
      aria-label="Workspace toolbar tabs"
    >
      {sections ? (
        sections.map((section, index) => {
          const pane = leaves.find((item) => item.id === section.id);
          if (!pane) return null;
          const focused = pane.id === (focusedLeaf?.id ?? sections[0]?.id);
          return (
            <div
              key={pane.id}
              className={styles.section}
              data-toolbar-pane-id={pane.id}
              data-focused={focused}
              style={
                {
                  "--toolbar-pane-left": `${section.left}px`,
                  "--toolbar-pane-width": `${section.width}px`,
                } as CSSProperties
              }
            >
              <PaneTabStrip
                onFitChange={onFitChange}
                leaf={{ ...pane, id: "toolbar" }}
                tabId={activeTabId ?? "workspace-navigation"}
                navigationEntries={
                  focused ? [...foreignEntries, ...entries] : []
                }
                selectedNavigationKey={focused ? selected : undefined}
                onActivateLeaf={activate}
                hidePaneActions
                ownerBySurface={Object.fromEntries(
                  pane.tabIds
                    .filter((key) => key in ownerBySurface)
                    .map((key) => [key, ownerBySurface[key]]),
                )}
                tablistLabel={
                  index === 0
                    ? "Workspace tabs"
                    : `Workspace tabs: pane ${index + 1}`
                }
              />
            </div>
          );
        })
      ) : (
        <div className={styles.fallback}>{strip}</div>
      )}
    </div>
  );
}
