import { addListener, isAnyOf } from "@reduxjs/toolkit";
import { type ReactNode, useEffect } from "react";

import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectCapabilities, selectHost } from "../Config/configSlice";
import { push, selectCurrentPage } from "../Pages/pagesSlice";
import { ActivityRail } from "./ActivityRail";
import { Dock } from "./Dock";
import { selectEditPlayer } from "./FilesPanel/filesPanelSlice";
import { isFileSurface, isGitDiffSurface, isGitSurface } from "./surfaceKey";
import { useAgentsSidebarAutoOpen } from "./useAgentsSidebarAutoOpen";
import { useWorkspaceShortcuts } from "./useWorkspaceShortcuts";
import {
  isDockSectionAvailable,
  resolveWorkspaceDockAvailability,
} from "./workspaceAvailability";
import {
  addSurfaceToPane,
  openSurfaceInNewTab,
  openTab,
  selectFocusedWorkspaceSurfaceKey,
  selectPanelsForced,
  selectWorkspaceDock,
} from "./workspaceSlice";
import styles from "./WorkspaceShell.module.css";

const opensInChatWorkspace = (surfaceKey: string): boolean =>
  isFileSurface(surfaceKey) ||
  isGitSurface(surfaceKey) ||
  isGitDiffSurface(surfaceKey);

function useWorkspaceSurfaceNavigation() {
  const dispatch = useAppDispatch();

  useEffect(
    () =>
      dispatch(
        addListener({
          matcher: isAnyOf(openTab, addSurfaceToPane, openSurfaceInNewTab),
          effect: (action, listenerApi) => {
            if (
              !openTab.match(action) &&
              !addSurfaceToPane.match(action) &&
              !openSurfaceInNewTab.match(action)
            )
              return;
            const surfaceKey = addSurfaceToPane.match(action)
              ? action.payload.surfaceKey
              : action.payload;
            if (!opensInChatWorkspace(surfaceKey)) return;
            const state = listenerApi.getState() as Parameters<
              typeof selectCurrentPage
            >[0] &
              Parameters<typeof selectEditPlayer>[0] &
              Parameters<typeof selectFocusedWorkspaceSurfaceKey>[0];
            if (selectCurrentPage(state)?.name === "chat") return;
            if (selectEditPlayer(state).status === "playing") return;
            if (selectFocusedWorkspaceSurfaceKey(state) !== surfaceKey) {
              return;
            }
            listenerApi.dispatch(push({ name: "chat" }));
          },
        }),
      ),
    [dispatch],
  );
}

export function WorkspaceShell({ children }: { children: ReactNode }) {
  useWorkspaceShortcuts();
  useAgentsSidebarAutoOpen();
  useWorkspaceSurfaceNavigation();
  const host = useAppSelector(selectHost);
  const capabilities = useAppSelector(selectCapabilities);
  const panelsForced = useAppSelector(selectPanelsForced);
  const dock = useAppSelector(selectWorkspaceDock);
  const dockMounted = isDockSectionAvailable(
    dock.section,
    resolveWorkspaceDockAvailability(host, capabilities, panelsForced),
  );

  return (
    <div className={styles.shell} data-testid="workspace-shell">
      <ActivityRail />
      {dockMounted ? <Dock /> : null}
      <div className={styles.content}>{children}</div>
    </div>
  );
}
