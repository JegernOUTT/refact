import { skipToken } from "@reduxjs/toolkit/query";
import { useCallback, useEffect, useRef } from "react";

import { useAppSelector } from "../../../../hooks";
import { useIsDocumentVisible } from "../../../../hooks/useIsDocumentVisible";
import {
  useGetGitFileQuery,
  type GitFileResponse,
} from "../../../../services/refact/gitRead";
import type { SurfaceKey } from "../../surfaceKey";
import { selectVisibleSurfaceKeys } from "../../workspaceSlice";
import {
  isMutableGitRev,
  type GitDiffDescriptor,
  type GitDiffSide,
} from "./gitDiffDescriptor";

export const GIT_DIFF_POLL_MS = 5000;

type SideQuery = {
  data: GitFileResponse | undefined;
  error: unknown;
  isLoading: boolean;
  isFetching: boolean;
  refetch: () => void;
};

function useSideQuery(
  root: string,
  side: GitDiffSide | null,
  pollingInterval: number,
): SideQuery {
  const query = useGetGitFileQuery(
    side ? { root, path: side.path, rev: side.rev } : skipToken,
    {
      pollingInterval: side && isMutableGitRev(side.rev) ? pollingInterval : 0,
      skipPollingIfUnfocused: true,
    },
  );
  const { refetch } = query;
  const safeRefetch = useCallback(() => {
    if (side) void refetch();
  }, [refetch, side]);
  return {
    data: side ? query.data : undefined,
    error: side ? query.error : undefined,
    isLoading: side ? query.isLoading : false,
    isFetching: side ? query.isFetching : false,
    refetch: safeRefetch,
  };
}

/**
 * Loads both diff sides. WORKTREE/INDEX sides poll and refetch on window
 * focus only while the tab is a visible surface and the document is
 * visible; HEAD and commit sides are immutable (HEAD is refreshed through
 * the GitFile tag invalidated by commits and other write operations).
 */
export function useGitDiffSides(
  descriptor: GitDiffDescriptor,
  surfaceKey: SurfaceKey | undefined,
) {
  const surfaceVisible = useAppSelector((state) =>
    surfaceKey ? selectVisibleSurfaceKeys(state).includes(surfaceKey) : true,
  );
  const refreshRef = useRef<() => void>(() => undefined);
  const documentVisible = useIsDocumentVisible(() => {
    if (surfaceVisible) refreshRef.current();
  });
  const active = surfaceVisible && documentVisible;
  const pollingInterval = active ? GIT_DIFF_POLL_MS : 0;

  const left = useSideQuery(descriptor.root, descriptor.left, pollingInterval);
  const right = useSideQuery(
    descriptor.root,
    descriptor.right,
    pollingInterval,
  );

  const leftMutable = Boolean(
    descriptor.left && isMutableGitRev(descriptor.left.rev),
  );
  const rightMutable = Boolean(
    descriptor.right && isMutableGitRev(descriptor.right.rev),
  );
  const refreshMutable = useCallback(() => {
    if (leftMutable) left.refetch();
    if (rightMutable) right.refetch();
  }, [left, leftMutable, right, rightMutable]);
  refreshRef.current = refreshMutable;

  useEffect(() => {
    if (!surfaceVisible || (!leftMutable && !rightMutable)) return;
    const onFocus = () => refreshRef.current();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [leftMutable, rightMutable, surfaceVisible]);

  const refreshAll = useCallback(() => {
    left.refetch();
    right.refetch();
  }, [left, right]);

  return { left, right, refreshAll };
}
