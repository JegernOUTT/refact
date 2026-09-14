import { useCallback, useEffect, useState } from "react";
import { useAppDispatch } from "../../../hooks";
import { useConfig } from "../../../hooks/useConfig";
import { buildApiUrl } from "../../../services/refact/apiUrl";
import { workApi, type WorkQuery } from "../../../services/refact/work";
import { subscribeToWork } from "../../../services/refact/workSubscription";
import { useGetCoordinationSettingsQuery } from "../../../services/refact/performance";

export function useWorkStream(query: WorkQuery, enabled: boolean) {
  const config = useConfig();
  const dispatch = useAppDispatch();
  const [error, setError] = useState<string | null>(null);
  const [epoch, setEpoch] = useState(0);
  const retry = useCallback(() => setEpoch((current) => current + 1), []);
  const { currentData: settings } = useGetCoordinationSettingsQuery("global");
  const maxBytes = settings?.effective.work_stream_max_bytes;
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

  useEffect(() => {
    if (
      !enabled ||
      typeof maxBytes !== "number" ||
      typeof timeoutMs !== "number" ||
      typeof retryMs !== "number"
    )
      return;
    let reconnect: ReturnType<typeof setTimeout> | undefined;
    const stop = subscribeToWork(url, token ?? null, {
      maxBytes,
      timeoutMs,
      onSnapshot: (page) => {
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
      onError: (reason) => {
        setError(reason);
        reconnect = setTimeout(retry, retryMs);
      },
    });
    return () => {
      stop();
      clearTimeout(reconnect);
    };
  }, [
    enabled,
    maxBytes,
    timeoutMs,
    retryMs,
    url,
    token,
    endpoint,
    chatId,
    states,
    dispatch,
    retry,
    epoch,
  ]);
  return { error, retry };
}
