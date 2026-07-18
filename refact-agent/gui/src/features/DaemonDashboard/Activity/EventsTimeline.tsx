import { useEffect, useMemo, useRef, useState } from "react";

import {
  Badge,
  Button,
  Select,
  Surface,
  VirtualList,
  type VirtualListHandle,
} from "../../../components/ui";
import { useAppSelector } from "../../../hooks";
import type {
  DaemonEvent,
  DaemonWorker,
} from "../../../services/refact/daemon";
import {
  selectDaemonEvents,
  selectDaemonStreamStatus,
} from "../dashboardSlice";
import { filterDaemonEvents, timelineFollowAfterScroll } from "./activityState";
import styles from "./ActivityPage.module.css";

const TIMESTAMP_FORMAT = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

function payloadSummary(payload: unknown): string {
  if (payload === null || payload === undefined) return "No payload";
  if (typeof payload === "string") return payload;
  if (typeof payload !== "object") return String(payload);
  const record = payload as Record<string, unknown>;
  for (const key of ["message", "reason", "status", "action", "state"]) {
    if (typeof record[key] === "string") return record[key];
  }
  try {
    return JSON.stringify(payload);
  } catch {
    return "Payload unavailable";
  }
}

function formatPayload(payload: unknown): string {
  if (payload === undefined) return "undefined";
  try {
    return JSON.stringify(payload, null, 2);
  } catch {
    return "Payload unavailable";
  }
}

type EventRowProps = {
  event: DaemonEvent;
  projectSlug: string | null;
};

function EventRow({ event, projectSlug }: EventRowProps) {
  return (
    <details className={styles.eventRow}>
      <summary className={styles.eventSummary}>
        <Badge size="xs" tone="accent" variant="soft">
          {event.kind}
        </Badge>
        {projectSlug ? (
          <span className={styles.projectSlug}>{projectSlug}</span>
        ) : null}
        <time
          className={styles.timestamp}
          dateTime={new Date(event.ts_ms).toISOString()}
        >
          {TIMESTAMP_FORMAT.format(event.ts_ms)}
        </time>
        <span className={styles.payloadSummary}>
          {payloadSummary(event.payload)}
        </span>
      </summary>
      <pre className={styles.payloadJson}>{formatPayload(event.payload)}</pre>
    </details>
  );
}

type EventsTimelineProps = {
  workers: DaemonWorker[];
};

export function EventsTimeline({ workers }: EventsTimelineProps) {
  const events = useAppSelector(selectDaemonEvents);
  const streamStatus = useAppSelector(selectDaemonStreamStatus);
  const [selectedKinds, setSelectedKinds] = useState<Set<string>>(new Set());
  const [projectId, setProjectId] = useState<string | null>(null);
  const [following, setFollowing] = useState(true);
  const listRef = useRef<VirtualListHandle>(null);
  const kinds = useMemo(
    () => [...new Set(events.map((event) => event.kind))].sort(),
    [events],
  );
  const projectSlugs = useMemo(
    () => new Map(workers.map((worker) => [worker.project_id, worker.slug])),
    [workers],
  );
  const visibleEvents = useMemo(
    () =>
      filterDaemonEvents(events, selectedKinds, projectId).sort(
        (left, right) => right.seq - left.seq,
      ),
    [events, projectId, selectedKinds],
  );
  const newestSequence = visibleEvents.length > 0 ? visibleEvents[0].seq : null;

  useEffect(() => {
    if (!following || newestSequence === null) return;
    const frame = requestAnimationFrame(() => {
      listRef.current?.scrollToIndex({ index: 0, align: "start" });
    });
    return () => cancelAnimationFrame(frame);
  }, [following, newestSequence]);

  function toggleKind(kind: string) {
    setSelectedKinds((current) => {
      const next = new Set(current);
      if (next.has(kind)) next.delete(kind);
      else next.add(kind);
      return next;
    });
  }

  const statusTone =
    streamStatus === "connected"
      ? "success"
      : streamStatus === "reconnecting"
        ? "warning"
        : "muted";

  return (
    <Surface className={styles.timelinePane} variant="glass" radius="card">
      <div className={styles.paneHeader}>
        <div>
          <h2>Events timeline</h2>
          <p>{visibleEvents.length} events in the current view</p>
        </div>
        <div className={styles.headerActions}>
          <Badge tone={statusTone} variant="soft">
            {streamStatus}
          </Badge>
          <Button
            aria-pressed={following}
            onClick={() => setFollowing((current) => !current)}
            size="sm"
            variant={following ? "soft" : "ghost"}
          >
            Follow {following ? "on" : "off"}
          </Button>
        </div>
      </div>
      <div className={styles.filters}>
        <Select
          value={projectId ?? "all"}
          onValueChange={(value) =>
            setProjectId(value === "all" ? null : value)
          }
        >
          <Select.Trigger aria-label="Filter events by project">
            <Select.Value />
          </Select.Trigger>
          <Select.Content>
            <Select.Item value="all">All projects</Select.Item>
            {workers.map((worker) => (
              <Select.Item key={worker.project_id} value={worker.project_id}>
                {worker.slug}
              </Select.Item>
            ))}
          </Select.Content>
        </Select>
        <div className={styles.kindFilters} aria-label="Filter events by kind">
          {kinds.map((kind) => {
            const selected = selectedKinds.has(kind);
            return (
              <label
                className={styles.kindFilter}
                data-selected={selected}
                key={kind}
              >
                <input
                  checked={selected}
                  onChange={() => toggleKind(kind)}
                  type="checkbox"
                />
                {kind}
              </label>
            );
          })}
        </div>
      </div>
      <div className={styles.timelineList}>
        <VirtualList
          className={styles.timelineVirtualized}
          emptyMessage="No events match the current filters."
          getItemKey={(event) => event.seq}
          height="100%"
          items={visibleEvents}
          listRef={listRef}
          onListScroll={(event) => {
            setFollowing((current) =>
              timelineFollowAfterScroll(current, event.currentTarget.scrollTop),
            );
          }}
          onListWheel={(event) => {
            if (event.deltaY > 0) setFollowing(false);
          }}
          renderItem={(event) => (
            <EventRow
              event={event}
              projectSlug={
                event.project_id
                  ? projectSlugs.get(event.project_id) ?? event.project_id
                  : null
              }
            />
          )}
        />
      </div>
    </Surface>
  );
}
