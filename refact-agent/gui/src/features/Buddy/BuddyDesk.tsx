import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ArrowLeft, RefreshCw } from "lucide-react";
import { Button, Text } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import {
  useGetBuddyOperatorArtifactQuery,
  useGetBuddyOperatorWorkQuery,
  useGetBuddyOperatorWorkItemQuery,
  buddyOperatorIdentity,
  type OperatorRecord,
} from "../../services/refact/buddyOperator";
import type { WorkItem, WorkItemState } from "./operatorTypes";
import { humanizeIdentifier } from "../../utils/displayNames";
import { useConfig } from "../../hooks/useConfig";
import { BuddyDeskActions } from "./BuddyDeskActions";
import { BuddyReportReadState } from "./BuddyReportReadState";
import { BuddyArtifactBody } from "./BuddyArtifactBody";
import { BuddyReportHistory } from "./BuddyReportHistory";
import { BuddyWorkSnooze } from "./BuddyWorkSnooze";
import styles from "./BuddyDesk.module.css";

const views = ["Needs you", "Working on", "Delivered", "Watching"] as const;
type WorkView = (typeof views)[number];
const viewQuery = {
  "Needs you": "needs_you",
  "Working on": "working_on",
  Delivered: "delivered",
  Watching: "watching",
} as const;

const viewForState: Record<WorkItemState, WorkView> = {
  needs_attention: "Needs you",
  blocked: "Needs you",
  working: "Working on",
  delivered: "Delivered",
  cancelled: "Delivered",
  watching: "Watching",
};

function WorkDetail({
  item,
  endpointIdentity,
  onBack,
}: {
  item: WorkItem;
  endpointIdentity: string;
  onBack: () => void;
}) {
  const { currentData, isFetching, error, refetch } =
    useGetBuddyOperatorArtifactQuery(
      {
        id: item.current_artifact_id ?? "",
        revision: item.current_artifact_revision ?? undefined,
        project_id: item.project.project_id,
        endpoint_identity: endpointIdentity,
      },
      { skip: !item.current_artifact_id },
    );
  const artifact = currentData?.data.value;
  const artifactMatches =
    artifact?.project.project_id === item.project.project_id &&
    artifact.project.canonical_root === item.project.canonical_root;
  const backButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    backButton.current?.focus();
  }, []);
  return (
    <article className={styles.detail} aria-label="Work detail">
      <Button
        ref={backButton}
        variant="ghost"
        size="sm"
        leftIcon={ArrowLeft}
        onClick={onBack}
      >
        Back to work list
      </Button>
      <h3>{item.title}</h3>
      <Text size="1">
        {humanizeIdentifier(item.state)} · {item.subject.resource_id}
      </Text>
      {!item.current_artifact_id && (
        <p>No report has been saved for this work yet.</p>
      )}
      {isFetching && <p role="status">Loading report…</p>}
      {error && (
        <div role="alert">
          <p>The saved report could not be loaded.</p>
          <Button size="sm" onClick={() => void refetch()}>
            Retry report
          </Button>
        </div>
      )}
      {artifact && !artifactMatches && (
        <p role="alert">The saved report belongs to another project.</p>
      )}
      {artifact && artifactMatches && !error && (
        <>
          <h4>{artifact.title}</h4>
          <p className={styles.conclusion}>{artifact.conclusion}</p>
          {artifact.delta && (
            <section aria-label="What changed">
              <h4>What changed</h4>
              <p>{artifact.delta}</p>
            </section>
          )}
          <BuddyArtifactBody body={artifact.body} />
          {artifact.uncertainty && (
            <section aria-label="Limitations">
              <h4>Limitations</h4>
              <p>{artifact.uncertainty}</p>
            </section>
          )}
          <BuddyReportReadState
            key={`read:${artifact.artifact_id}:${artifact.revision}`}
            item={item}
            artifact={artifact}
            endpointIdentity={endpointIdentity}
          />
          <BuddyDeskActions
            item={item}
            artifact={artifact}
            endpointIdentity={endpointIdentity}
          />
          <BuddyWorkSnooze
            key={`snooze:${artifact.artifact_id}:${artifact.revision}`}
            item={item}
            artifact={artifact}
            identity={endpointIdentity}
          />
          <details>
            <summary>Evidence · revision {artifact.revision}</summary>
            {artifact.evidence.length === 0 ? (
              <p>No evidence references were recorded.</p>
            ) : (
              <ul>
                {artifact.evidence.map((evidence) => (
                  <li key={evidence.evidence_id}>
                    <strong>
                      {humanizeIdentifier(evidence.source.source_kind)}
                    </strong>{" "}
                    · {evidence.source.source_id}
                    <div>{evidence.locator}</div>
                    <Text size="1">Observed {evidence.observed_at}</Text>
                  </li>
                ))}
              </ul>
            )}
          </details>
          <BuddyReportHistory
            item={item}
            artifact={artifact}
            identity={endpointIdentity}
          />
        </>
      )}
    </article>
  );
}

function DeskContent({
  endpointIdentity,
  workId,
  projectId,
}: {
  endpointIdentity: string;
  workId?: string;
  projectId?: string;
}) {
  const config = useConfig();
  const [cursor, setCursor] = useState<number>();
  const [previous, setPrevious] = useState<OperatorRecord<WorkItem>[]>([]);
  const [view, setView] = useState<WorkView>("Needs you");
  const [selectedId, setSelectedId] = useState<string | null>(workId ?? null);
  const linkedWork = useGetBuddyOperatorWorkItemQuery(
    {
      id: workId ?? "",
      project_id: projectId,
      endpoint_identity: endpointIdentity,
    },
    { skip: !workId || !projectId, refetchOnMountOrArgChange: true },
  );
  const [search, setSearch] = useState("");
  const [attention, setAttention] = useState<
    "all" | "unread" | "snoozed" | "unsnoozed"
  >("all");
  const trigger = useRef<HTMLButtonElement | null>(null);
  const { currentData, isFetching, error, refetch } =
    useGetBuddyOperatorWorkQuery(
      {
        cursor,
        limit: 50,
        endpoint_identity: endpointIdentity,
        view: viewQuery[view],
        search: search.trim(),
        attention,
      },
      { refetchOnMountOrArgChange: true },
    );
  const compatiblePrevious = currentData
    ? previous.filter(
        (record) =>
          record.value.project.project_id === currentData.project.project_id &&
          record.value.project.canonical_root ===
            currentData.project.canonical_root,
      )
    : previous;
  const records = [
    ...new Map(
      [...compatiblePrevious, ...(currentData?.data ?? [])].map((record) => [
        record.value.id,
        record,
      ]),
    ).values(),
  ];
  const selected =
    selectedId === workId
      ? linkedWork.currentData?.data.value
      : records.find((record) => record.value.id === selectedId)?.value;
  const query = search.trim().toLocaleLowerCase();
  const filtered = records.filter(
    ({ value }) => viewForState[value.state] === view,
  );

  useEffect(() => {
    if (
      currentData &&
      previous.some(
        (record) =>
          record.value.project.canonical_root !==
            currentData.project.canonical_root ||
          record.value.project.project_id !== currentData.project.project_id,
      )
    ) {
      setPrevious([]);
      setSelectedId(null);
      setCursor(undefined);
    }
  }, [currentData, previous]);

  useLayoutEffect(() => {
    if (selectedId === null) trigger.current?.focus();
  }, [selectedId]);

  const refresh = () => {
    setPrevious([]);
    setSelectedId(null);
    if (cursor === undefined) void refetch();
    else setCursor(undefined);
  };
  const back = () => {
    setSelectedId(null);
    if (config.host === "web" && config.engineServed) {
      const url = new URL(window.location.href);
      url.searchParams.delete("buddy_work");
      url.searchParams.delete("buddy_project");
      window.history.replaceState(window.history.state, "", url);
    }
  };
  const updateLink = (item: WorkItem) => {
    if (config.host !== "web" || !config.engineServed) return;
    const url = new URL(window.location.href);
    url.searchParams.set("buddy_work", item.id);
    url.searchParams.set("buddy_project", item.project.project_id);
    url.searchParams.delete("chat");
    window.history.replaceState(window.history.state, "", url);
  };

  return (
    <section
      className={styles.desk}
      aria-label="Buddy Desk"
      data-detail-open={Boolean(selected)}
    >
      <header className={styles.header}>
        <div>
          <h2>Buddy Desk</h2>
          <Text size="1">
            Results and decisions, without opening a worker chat.
          </Text>
        </div>
        <Button
          size="sm"
          variant="ghost"
          leftIcon={RefreshCw}
          disabled={isFetching}
          onClick={refresh}
        >
          Refresh work
        </Button>
      </header>
      {selectedId === workId && linkedWork.isFetching && (
        <p role="status">Loading linked work…</p>
      )}
      {selectedId === workId && linkedWork.error && (
        <div role="alert">
          <p>
            The linked work is unavailable in this project. No other report was
            substituted.
          </p>
          <Button size="sm" onClick={() => void linkedWork.refetch()}>
            Retry linked work
          </Button>
          <Button size="sm" variant="ghost" onClick={back}>
            Back to work list
          </Button>
        </div>
      )}
      <div className={styles.layout}>
        <div className={styles.list}>
          <nav className={styles.views} aria-label="Work views">
            {views.map((label) => (
              <Button
                key={label}
                size="sm"
                variant={view === label ? "primary" : "ghost"}
                aria-pressed={view === label}
                onClick={() => {
                  setView(label);
                  setSelectedId(null);
                  setCursor(undefined);
                  setPrevious([]);
                }}
              >
                {label}
              </Button>
            ))}
          </nav>
          <label className={styles.search}>
            Search saved work
            <input
              type="search"
              value={search}
              maxLength={256}
              onChange={(event) => {
                setSearch(event.target.value);
                setCursor(undefined);
                setPrevious([]);
                setSelectedId(null);
              }}
            />
          </label>
          <label className={styles.search}>
            Report attention
            <select
              value={attention}
              onChange={(event) => {
                const value = event.target.value;
                if (
                  value !== "all" &&
                  value !== "unread" &&
                  value !== "snoozed" &&
                  value !== "unsnoozed"
                )
                  return;
                setAttention(value);
                setCursor(undefined);
                setPrevious([]);
                setSelectedId(null);
              }}
            >
              <option value="all">All saved work</option>
              <option value="unread">Unread reports</option>
              <option value="snoozed">Snoozed reports</option>
              <option value="unsnoozed">Not snoozed</option>
            </select>
          </label>
          {isFetching && <p role="status">Loading work…</p>}
          {error ? (
            <div role="alert">
              <p>
                Buddy’s saved work is unavailable. Existing chat controls and
                settings remain available below.
              </p>
              <Button size="sm" onClick={refresh}>
                Retry work
              </Button>
            </div>
          ) : (
            <>
              {currentData && filtered.length === 0 && (
                <p>
                  No {view.toLocaleLowerCase()} items
                  {query ? " match this search" : " match these filters"}.
                </p>
              )}
              <ul className={styles.items}>
                {filtered.map(({ value }) => (
                  <li key={value.id}>
                    <button
                      className={styles.item}
                      aria-current={
                        selectedId === value.id ? "true" : undefined
                      }
                      onClick={(event) => {
                        trigger.current = event.currentTarget;
                        setSelectedId(value.id);
                        updateLink(value);
                      }}
                    >
                      <strong>{value.title}</strong>
                      <span>{humanizeIdentifier(value.state)}</span>
                      <span>{value.subject.resource_id}</span>
                    </button>
                  </li>
                ))}
              </ul>
              {currentData?.next_cursor != null && (
                <Button
                  size="sm"
                  disabled={isFetching}
                  onClick={() => {
                    setPrevious(records);
                    setCursor(currentData.next_cursor ?? undefined);
                  }}
                >
                  Load more work
                </Button>
              )}
            </>
          )}
        </div>
        {selected && !(selectedId === workId ? linkedWork.error : error) ? (
          <WorkDetail
            key={`${selected.id}:${selected.current_artifact_revision ?? 0}`}
            item={selected}
            endpointIdentity={endpointIdentity}
            onBack={back}
          />
        ) : (
          <p className={styles.placeholder}>
            Choose a work item to read its result and evidence.
          </p>
        )}
      </div>
    </section>
  );
}

export function BuddyDesk({
  workId,
  projectId,
}: {
  workId?: string;
  projectId?: string;
}) {
  const identity = useAppSelector(buddyOperatorIdentity);
  return (
    <DeskContent
      key={`${identity}:${projectId ?? ""}:${workId ?? ""}`}
      endpointIdentity={identity}
      workId={workId}
      projectId={projectId}
    />
  );
}
