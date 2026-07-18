import { useEffect, useRef } from "react";

import {
  daemonEventsReceived,
  daemonEventsReset,
  daemonStreamStatusChanged,
  selectDaemonEvents,
} from "../features/DaemonDashboard/dashboardSlice";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import { useConfig } from "./useConfig";
import {
  resolveDaemonBaseUrl,
  type DaemonEvent,
  useLazyGetDaemonEventsQuery,
} from "../services/refact/daemon";

const MAX_RECONNECT_DELAY_MS = 10_000;
const INITIAL_RECONNECT_DELAY_MS = 250;

export type DaemonEventsStreamOptions = {
  backfill?: (afterSequence: number) => Promise<DaemonEvent[]>;
  daemonStartedAtMs?: number | null;
};

type FlushHandle =
  | { type: "frame"; id: number }
  | { type: "timeout"; id: ReturnType<typeof setTimeout> };

function scheduleFrame(callback: () => void): FlushHandle {
  if (typeof globalThis.requestAnimationFrame === "function") {
    return {
      type: "frame",
      id: globalThis.requestAnimationFrame(callback),
    };
  }
  return { type: "timeout", id: setTimeout(callback, 16) };
}

function cancelFrame(handle: FlushHandle) {
  if (handle.type === "frame") {
    globalThis.cancelAnimationFrame(handle.id);
    return;
  }
  clearTimeout(handle.id);
}

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

export function useDaemonEventsStream(options: DaemonEventsStreamOptions = {}) {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const events = useAppSelector(selectDaemonEvents);
  const [getBackfill] = useLazyGetDaemonEventsQuery();
  const backfillOverride = options.backfill;
  const initialSequence = events.at(-1)?.seq ?? 0;
  const lastSequenceRef = useRef(initialSequence);
  const daemonStartedAtRef = useRef(options.daemonStartedAtMs);

  useEffect(() => {
    lastSequenceRef.current = Math.max(
      lastSequenceRef.current,
      initialSequence,
    );
  }, [initialSequence]);

  useEffect(() => {
    const previous = daemonStartedAtRef.current;
    const current = options.daemonStartedAtMs;
    daemonStartedAtRef.current = current;
    if (previous == null || current == null || previous === current) return;
    lastSequenceRef.current = 0;
    dispatch(daemonEventsReset());
  }, [dispatch, options.daemonStartedAtMs]);

  useEffect(() => {
    let eventSource: EventSource | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let flushHandle: FlushHandle | null = null;
    let stopped = false;
    let reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
    let connectGeneration = 0;
    let bufferedEvents: DaemonEvent[] = [];

    function flushEvents() {
      flushHandle = null;
      if (stopped || bufferedEvents.length === 0) return;
      const eventsToDispatch = bufferedEvents;
      bufferedEvents = [];
      dispatch(daemonEventsReceived(eventsToDispatch));
    }

    function bufferEvent(event: DaemonEvent) {
      bufferedEvents.push(event);
      flushHandle ??= scheduleFrame(flushEvents);
    }

    function connectionIsCurrent(generation: number) {
      return !stopped && generation === connectGeneration;
    }

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
        void connect(true);
      }, delay);
    }

    function handleMessage(message: MessageEvent<string>) {
      const event = parseDaemonEvent(message.data);
      if (!event || event.seq <= lastSequenceRef.current) return;

      lastSequenceRef.current = event.seq;
      bufferEvent(event);
    }

    async function connect(reconnecting: boolean) {
      if (stopped) return;
      const generation = ++connectGeneration;
      dispatch(
        daemonStreamStatusChanged(reconnecting ? "reconnecting" : "connecting"),
      );
      try {
        const backfill = backfillOverride
          ? await backfillOverride(lastSequenceRef.current)
          : await getBackfill(lastSequenceRef.current, false).unwrap();
        if (!connectionIsCurrent(generation)) return;
        if (backfill.length > 0) {
          lastSequenceRef.current = Math.max(
            lastSequenceRef.current,
            ...backfill.map((event) => event.seq),
          );
          dispatch(daemonEventsReceived(backfill));
        }
      } catch {
        if (!connectionIsCurrent(generation)) return;
      }
      const params = new URLSearchParams({
        after_seq: String(lastSequenceRef.current),
        follow: "true",
      });
      const url = `${resolveDaemonBaseUrl(
        config,
      )}/daemon/v1/events?${params.toString()}`;
      eventSource = new EventSource(url);
      eventSource.onopen = () => {
        reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
        dispatch(daemonStreamStatusChanged("connected"));
      };
      eventSource.onmessage = handleMessage;
      eventSource.addEventListener("daemon", handleMessage as EventListener);
      eventSource.onerror = () => scheduleReconnect(false);
    }

    void connect(false);

    return () => {
      stopped = true;
      connectGeneration += 1;
      if (reconnectTimer !== null) clearTimeout(reconnectTimer);
      if (flushHandle !== null) cancelFrame(flushHandle);
      eventSource?.close();
    };
  }, [
    backfillOverride,
    config,
    dispatch,
    getBackfill,
    options.daemonStartedAtMs,
  ]);
}
