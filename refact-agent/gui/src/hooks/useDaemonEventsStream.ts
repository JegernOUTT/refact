import { useEffect, useRef } from "react";

import {
  daemonEventsReceived,
  daemonStreamStatusChanged,
  selectDaemonEvents,
} from "../features/DaemonDashboard/dashboardSlice";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import { useConfig } from "./useConfig";
import {
  resolveDaemonBaseUrl,
  type DaemonEvent,
} from "../services/refact/daemon";

const MAX_RECONNECT_DELAY_MS = 10_000;
const INITIAL_RECONNECT_DELAY_MS = 250;

function parseDaemonEvent(data: string): DaemonEvent | null {
  try {
    const event = JSON.parse(data) as Partial<DaemonEvent>;
    if (
      typeof event.seq !== "number" ||
      typeof event.ts_ms !== "number" ||
      typeof event.kind !== "string"
    ) {
      return null;
    }
    return {
      seq: event.seq,
      ts_ms: event.ts_ms,
      kind: event.kind,
      project_id:
        typeof event.project_id === "string" ? event.project_id : null,
      payload: event.payload ?? null,
    };
  } catch {
    return null;
  }
}

export function useDaemonEventsStream() {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const events = useAppSelector(selectDaemonEvents);
  const initialSequence = events.at(-1)?.seq ?? 0;
  const lastSequenceRef = useRef(initialSequence);

  useEffect(() => {
    lastSequenceRef.current = Math.max(
      lastSequenceRef.current,
      initialSequence,
    );
  }, [initialSequence]);

  useEffect(() => {
    let eventSource: EventSource | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let stopped = false;
    let reconnectDelay = INITIAL_RECONNECT_DELAY_MS;

    function scheduleReconnect(immediate: boolean) {
      eventSource?.close();
      eventSource = null;
      if (stopped || reconnectTimer !== null) return;
      dispatch(daemonStreamStatusChanged("reconnecting"));
      const delay = immediate ? 0 : reconnectDelay;
      if (!immediate) {
        reconnectDelay = Math.min(reconnectDelay * 2, MAX_RECONNECT_DELAY_MS);
      }
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null;
        connect();
      }, delay);
    }

    function handleMessage(message: MessageEvent<string>) {
      const event = parseDaemonEvent(message.data);
      if (!event || event.seq <= lastSequenceRef.current) return;

      const expectedSequence = lastSequenceRef.current + 1;
      const resyncMarker = event.kind === "daemon_events_resync_required";
      if (event.seq !== expectedSequence && !resyncMarker) {
        scheduleReconnect(true);
        return;
      }

      lastSequenceRef.current = event.seq;
      dispatch(daemonEventsReceived([event]));
    }

    function connect() {
      if (stopped) return;
      const params = new URLSearchParams({
        after_seq: String(lastSequenceRef.current),
        follow: "true",
      });
      const url = `${resolveDaemonBaseUrl(
        config,
      )}/daemon/v1/events?${params.toString()}`;
      dispatch(daemonStreamStatusChanged("connecting"));
      eventSource = new EventSource(url);
      eventSource.onopen = () => {
        reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
        dispatch(daemonStreamStatusChanged("connected"));
      };
      eventSource.onmessage = handleMessage;
      eventSource.addEventListener("daemon", handleMessage as EventListener);
      eventSource.onerror = () => scheduleReconnect(false);
    }

    connect();

    return () => {
      stopped = true;
      if (reconnectTimer !== null) clearTimeout(reconnectTimer);
      eventSource?.close();
    };
  }, [config, dispatch]);
}
