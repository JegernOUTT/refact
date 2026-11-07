import { useCallback, useEffect, useRef, useState } from "react";
import { useAppDispatch } from "../../../hooks";
import { useConfig } from "../../../hooks/useConfig";
import { buildApiUrl } from "../../../services/refact/apiUrl";
import { workApi, type WorkQuery } from "../../../services/refact/work";
import { subscribeToWork } from "../../../services/refact/workSubscription";
import type { SnapshotFailure } from "../../../services/refact/snapshotError";
import { useGetCoordinationSettingsQuery } from "../../../services/refact/performance";

export function useWorkStream(
  query: WorkQuery,
  enabled: boolean,
  snapshotVersion?: number,
) {
  const config = useConfig();
  const dispatch = useAppDispatch();
  const [error, setError] = useState<SnapshotFailure | null>(null);
  const [blocked, setBlocked] = useState(false);
  const attempts = useRef(0);
  const denied = useRef(false);
  const recoveryRequested = useRef(false);
  const [reconnecting, setReconnecting] = useState(false);
  const [epoch, setEpoch] = useState(0);
  const reconnect = useCallback(() => setEpoch((current) => current + 1), []);
  const retry = useCallback(() => {
    attempts.current = 0;
    recoveryRequested.current = true;
    reconnect();
  }, [reconnect]);
  const { currentData: settings } = useGetCoordinationSettingsQuery("global");
  const timeoutMs = settings?.effective.work_stream_timeout_ms;
  const retryMs = settings?.effective.work_reconnect_max_ms;
  const url = buildApiUrl(config, "/v1/work/subscribe", {
    chat_id: query.chatId,
    states: query.states?.join(","),
  });
  const token = config.apiKey;
  const endpoint = query.endpoint;
  const chatId = query.chatId;
  const states = query.states;
  const source = JSON.stringify([endpoint, chatId, states]);
  const currentSource = useRef(source);
  if (currentSource.current !== source) {
    currentSource.current = source;
    denied.current = false;
    recoveryRequested.current = false;
    attempts.current = 0;
    setError(null);
    setBlocked(false);
    setReconnecting(false);
  }

  useEffect(() => {
    if (denied.current && !recoveryRequested.current) return;
    denied.current = false;
    recoveryRequested.current = false;
    attempts.current = 0;
    setError(null);
    setBlocked(false);
    setReconnecting(false);
  }, [snapshotVersion]);

  useEffect(() => {
    if (
      !enabled ||
      typeof timeoutMs !== "number" ||
      typeof retryMs !== "number" ||
      !Number.isFinite(timeoutMs) ||
      timeoutMs <= 0 ||
      !Number.isFinite(retryMs) ||
      retryMs <= 0
    )
      return;
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const stop = subscribeToWork(url, token ?? null, {
      timeoutMs,
      onSnapshot: (page) => {
        if (!active || currentSource.current !== source) return;
        denied.current = false;
        recoveryRequested.current = false;
        setReconnecting(false);
        setBlocked(false);
        setError(null);
        dispatch(
          workApi.util.updateQueryData(
            "getWork",
            { endpoint, chatId, states },
            (draft) => {
              if (
                draft.snapshot_id === page.snapshot_id &&
                draft.seq > page.seq
              )
                return;
              return page;
            },
          ),
        );
      },
      onError: (
        _reason,
        failure = { kind: "visibility", data: { code: "policy_unavailable" } },
      ) => {
        if (!active || currentSource.current !== source) return;
        active = false;
        if (failure.kind !== "transport") {
          denied.current = true;
          recoveryRequested.current = false;
          dispatch(
            workApi.util.getRunningQueryThunk("getWork", {
              endpoint,
              chatId,
              states,
            }),
          )?.abort();
          setBlocked(true);
          dispatch(
            workApi.util.updateQueryData(
              "getWork",
              { endpoint, chatId, states },
              (draft) => {
                draft.rows = [];
                draft.roots = [];
                draft.total = 0;
                draft.returned = 0;
                delete draft.next_cursor;
                delete draft.withheld_rows;
              },
            ),
          );
        }
        setError((previous) =>
          previous &&
          previous.kind !== "transport" &&
          failure.kind === "transport"
            ? previous
            : failure,
        );
        const shouldReconnect =
          (failure.kind === "transport" || failure.kind === "reset") &&
          attempts.current < 3;
        setReconnecting(shouldReconnect);
        if (shouldReconnect) {
          attempts.current += 1;
          timer = setTimeout(reconnect, retryMs);
        }
      },
    });
    return () => {
      active = false;
      stop();
      clearTimeout(timer);
    };
  }, [
    enabled,
    timeoutMs,
    retryMs,
    url,
    token,
    endpoint,
    chatId,
    states,
    source,
    dispatch,
    reconnect,
    epoch,
  ]);
  return { error, retry, reconnecting, blocked };
}
