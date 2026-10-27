import { skipToken } from "@reduxjs/toolkit/query";
import { useCallback, useEffect, useRef } from "react";

import { useAppSelector } from "../../../../hooks";
import { useIsDocumentVisible } from "../../../../hooks/useIsDocumentVisible";
import {
  useGetGitStatusQuery,
  useGetGitFileQuery,
  type GitFileResponse,
} from "../../../../services/refact/gitRead";
import type { SurfaceKey } from "../../surfaceKey";
import { selectVisibleSurfaceKeys } from "../../workspaceSlice";
import { GIT_STATUS_POLL_MS } from "../useGitStatus";
import {
  isMutableGitRev,
  type GitDiffDescriptor,
  type GitDiffSide,
} from "./gitDiffDescriptor";

export const GIT_DIFF_POLL_MS = 5000;

/** Revs whose content depends on where HEAD points. */
const HEAD_RELATIVE_REVS: ReadonlySet<string> = new Set(["HEAD", "INDEX"]);

const PENDING_HEAD = Symbol("pending-head");
type HeadKey = string | null | typeof PENDING_HEAD;

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
  headKey: HeadKey,
): SideQuery {
  const headRelative = side !== null && HEAD_RELATIVE_REVS.has(side.rev);
  // HEAD/INDEX sides wait for the status head oid so they are cached per
  // HEAD: a commit or checkout made outside the app moves the oid and the
  // side refetches instead of showing the old HEAD.
  const waiting = headRelative && headKey === PENDING_HEAD;
  const query = useGetGitFileQuery(
    side && !waiting
      ? {
          root,
          path: side.path,
          rev: side.rev,
          ...(headRelative && headKey !== PENDING_HEAD
            ? { headOid: headKey }
            : {}),
        }
      : skipToken,
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
    isLoading: side ? waiting || query.isLoading : false,
    isFetching: side ? query.isFetching : false,
    refetch: safeRefetch,
  };
}

/**
 * The HEAD oid of `root` from a status query that polls while the diff is
 * visible. A failed status read falls back to `null` so sides still load.
 */
function useHeadKey(root: string, needed: boolean, active: boolean): HeadKey {
  const status = useGetGitStatusQuery(needed ? [root] : skipToken, {
    pollingInterval: active ? GIT_STATUS_POLL_MS : 0,
    skipPollingIfUnfocused: true,
  });
  if (!needed) return null;
  if (status.data) {
    const entry =
      status.data.roots.find((item) => item.root === root) ??
      status.data.roots.at(0);
    return entry?.head_oid ?? null;
  }
  return status.isError ? null : PENDING_HEAD;
}

/**
 * Loads both diff sides. WORKTREE/INDEX sides poll and refetch on window
 * focus only while the tab is a visible surface and the document is
 * visible; commit sides are immutable. HEAD/INDEX sides are keyed by the
 * current HEAD oid (from a polled status query), so HEAD moving outside the
 * app refetches them; in-app writes also invalidate the GitFile tag.
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

  const needsHead = [descriptor.left, descriptor.right].some(
    (side) => side !== null && HEAD_RELATIVE_REVS.has(side.rev),
  );
  const headKey = useHeadKey(descriptor.root, needsHead, active);
  const left = useSideQuery(
    descriptor.root,
    descriptor.left,
    pollingInterval,
    headKey,
  );
  const right = useSideQuery(
    descriptor.root,
    descriptor.right,
    pollingInterval,
    headKey,
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
