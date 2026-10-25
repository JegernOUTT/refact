import { useCallback, useEffect, useState } from "react";
import { useAppDispatch } from "../../../hooks";
import { useConfig } from "../../../hooks/useConfig";
import { buildApiUrl } from "../../../services/refact/apiUrl";
import { causalApi, type CausalQuery } from "../../../services/refact/causal";
import { subscribeToCausalTrace } from "../../../services/refact/causalSubscription";
import { useGetCoordinationSettingsQuery } from "../../../services/refact/performance";

export function useCausalStream(query: CausalQuery, enabled: boolean) {
  const config = useConfig();
  const dispatch = useAppDispatch();
  const [error, setError] = useState<string | null>(null);
  const [epoch, setEpoch] = useState(0);
  const retry = useCallback(() => setEpoch((current) => current + 1), []);
  const { currentData: settings } = useGetCoordinationSettingsQuery("global");
  const timeoutMs = settings?.effective.work_stream_timeout_ms;
  const retryMs = settings?.effective.work_reconnect_max_ms;
  const url = buildApiUrl(config, "/v1/work/causal/subscribe", {
    root: query.root,
    chat_id: query.chatId,
  });
  const token = config.apiKey;
  const endpoint = query.endpoint;
  const chatId = query.chatId;
  const root = query.root;

  useEffect(() => {
    if (
      !enabled ||
      typeof timeoutMs !== "number" ||
      typeof retryMs !== "number"
    )
      return;
    let reconnect: ReturnType<typeof setTimeout> | undefined;
    const stop = subscribeToCausalTrace(url, token ?? null, {
      timeoutMs,
      onSnapshot: (page) => {
        setError(null);
        dispatch(
          causalApi.util.updateQueryData(
            "getCausalTrace",
            { endpoint, root, chatId },
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
    timeoutMs,
    retryMs,
    url,
    token,
    endpoint,
    chatId,
    root,
    dispatch,
    retry,
    epoch,
  ]);
  return { error, retry };
}
