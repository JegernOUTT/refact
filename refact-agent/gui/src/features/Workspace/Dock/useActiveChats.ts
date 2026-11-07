import { skipToken } from "@reduxjs/toolkit/query";
import { useEffect, useMemo, useState } from "react";

import { useAppSelector } from "../../../hooks";
import {
  chatActivityIdentity,
  useListChatActivityQuery,
} from "../../../services/refact/trajectories";
import {
  selectBackendStatus,
  selectBrowserOnline,
} from "../../Connection/connectionSlice";
import {
  activeChatsNeedAttention,
  buildActiveChatEntries,
  type ActiveChatEntry,
} from "./activeChatsModel";

export const ACTIVE_CHATS_POLL_MS = 15_000;

export type ActiveChatsStaleReason = "offline" | "refresh_failed" | null;

export type ActiveChatsState = {
  entries: ActiveChatEntry[];
  isLoading: boolean;
  isError: boolean;
  isStale: boolean;
  staleReason: ActiveChatsStaleReason;
  isOfflineWithoutData: boolean;
  isFetching: boolean;
  paused: boolean;
  refetch: () => void;
};

export function useActiveChats(enabled = true): ActiveChatsState {
  const [clock, setClock] = useState(Date.now);
  const browserOnline = useAppSelector(selectBrowserOnline);
  const backendStatus = useAppSelector(selectBackendStatus);
  const identity = useAppSelector(chatActivityIdentity);
  const paused = !browserOnline || backendStatus === "offline";
  const {
    currentData,
    isLoading,
    isError,
    isFetching,
    isUninitialized,
    refetch,
  } = useListChatActivityQuery(enabled ? { identity } : skipToken, {
    pollingInterval: paused ? 0 : ACTIVE_CHATS_POLL_MS,
    skipPollingIfUnfocused: true,
  });
  const entries = useMemo(
    () =>
      buildActiveChatEntries(currentData ?? [], Math.max(clock, Date.now())),
    [currentData, clock],
  );
  const hasAttention = activeChatsNeedAttention(entries);
  useEffect(() => {
    if (!enabled || !hasAttention) return;
    const refreshClock = () => {
      if (!document.hidden) setClock(Date.now());
    };
    const timer = window.setInterval(refreshClock, ACTIVE_CHATS_POLL_MS);
    document.addEventListener("visibilitychange", refreshClock);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshClock);
    };
  }, [enabled, hasAttention]);
  const hasData = currentData !== undefined;
  const isOfflineWithoutData = !hasData && paused;
  const staleReason: ActiveChatsStaleReason = !hasData
    ? null
    : paused
      ? "offline"
      : isError
        ? "refresh_failed"
        : null;
  return {
    entries,
    isLoading:
      !isOfflineWithoutData &&
      (isLoading || (!hasData && (isFetching || isUninitialized))),
    isError,
    isStale: staleReason !== null,
    staleReason,
    isOfflineWithoutData,
    isFetching,
    paused,
    refetch: () => {
      void refetch();
    },
  };
}
