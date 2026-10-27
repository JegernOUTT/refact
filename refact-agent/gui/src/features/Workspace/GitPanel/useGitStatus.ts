import { useEffect, useRef } from "react";

import { useAppSelector } from "../../../hooks";
import { useIsDocumentVisible } from "../../../hooks/useIsDocumentVisible";
import { useGetGitStatusQuery } from "../../../services/refact/gitRead";
import { isGitDiffSurface, isGitSurface } from "../surfaceKey";
import {
  selectFocusedChatWorkspaceRoots,
  selectVisibleSurfaceKeys,
  selectWorkspaceDock,
} from "../workspaceSlice";

export const GIT_STATUS_POLL_MS = 5000;

export function useGitStatusPollingActive(): boolean {
  const dock = useAppSelector(selectWorkspaceDock);
  const surfaceVisible = useAppSelector((state) =>
    selectVisibleSurfaceKeys(state).some(
      (key) => isGitSurface(key) || isGitDiffSurface(key),
    ),
  );
  return (dock.open && dock.section === "git") || surfaceVisible;
}

export function useGitStatus() {
  const contextRoots = useAppSelector(selectFocusedChatWorkspaceRoots);
  const active = useGitStatusPollingActive();
  const refetchRef = useRef<() => void>(() => undefined);
  const visible = useIsDocumentVisible(() => {
    if (active) refetchRef.current();
  });
  const query = useGetGitStatusQuery(contextRoots, {
    pollingInterval: visible && active ? GIT_STATUS_POLL_MS : 0,
  });
  const { refetch } = query;
  refetchRef.current = () => void refetch();

  useEffect(() => {
    if (!active) return;
    const onFocus = () => void refetch();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [active, refetch]);

  return query;
}
