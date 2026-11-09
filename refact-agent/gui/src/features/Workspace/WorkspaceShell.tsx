import { addListener, isAnyOf } from "@reduxjs/toolkit";
import { type ReactNode, useEffect, useRef } from "react";

import { useMediaQuery } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectCapabilities, selectHost } from "../Config/configSlice";
import { push, selectCurrentPage } from "../Pages/pagesSlice";
import { ActivityRail } from "./ActivityRail";
import { Dock } from "./Dock";
import { selectEditPlayer } from "./FilesPanel/filesPanelSlice";
import {
  isFileSurface,
  isGitDiffSurface,
  isGitSurface,
  isWorktreeDiffSurface,
} from "./surfaceKey";
import { useWorkspaceShortcuts } from "./useWorkspaceShortcuts";
import { WORKSPACE_NARROW_QUERY } from "./workspaceBreakpoints";
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
  setDockOpen,
} from "./workspaceSlice";
import styles from "./WorkspaceShell.module.css";

const opensInChatWorkspace = (surfaceKey: string): boolean =>
  isFileSurface(surfaceKey) ||
  isGitSurface(surfaceKey) ||
  isGitDiffSurface(surfaceKey) ||
  isWorktreeDiffSurface(surfaceKey);

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

export function WorkspaceShell({
  children,
  toolbar,
}: {
  children: ReactNode;
  toolbar?: ReactNode;
}) {
  useWorkspaceShortcuts();
  useWorkspaceSurfaceNavigation();
  const dispatch = useAppDispatch();
  const host = useAppSelector(selectHost);
  const capabilities = useAppSelector(selectCapabilities);
  const panelsForced = useAppSelector(selectPanelsForced);
  const dock = useAppSelector(selectWorkspaceDock);
  const isNarrow = useMediaQuery(WORKSPACE_NARROW_QUERY);
  const wasNarrow = useRef(false);
  const enteringNarrow = isNarrow && !wasNarrow.current;
  useEffect(() => {
    if (isNarrow && !wasNarrow.current) dispatch(setDockOpen(false));
    wasNarrow.current = isNarrow;
  }, [dispatch, isNarrow]);
  const dockMounted = isDockSectionAvailable(
    dock.section,
    resolveWorkspaceDockAvailability(host, capabilities, panelsForced),
  );

  return (
    <div className={styles.shell} data-testid="workspace-shell">
      <ActivityRail />
      {dockMounted ? <Dock enteringNarrow={enteringNarrow} /> : null}
      <div className={styles.content}>
        {toolbar}
        {children}
      </div>
    </div>
  );
}
