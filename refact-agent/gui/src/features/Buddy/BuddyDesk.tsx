import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ChevronRight,
  ClipboardList,
  FileSearch,
  Inbox,
  RefreshCw,
} from "lucide-react";
import { Button, Icon, Surface, Text } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import {
  useGetBuddyDeskActionsQuery,
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

const DEFAULT_VIEW: WorkView = "Needs you";

const viewForState: Record<WorkItemState, WorkView> = {
  needs_attention: "Needs you",
  blocked: "Needs you",
  working: "Working on",
  delivered: "Delivered",
  cancelled: "Delivered",
  watching: "Watching",
};

const emptyView: Record<WorkView, string> = {
  "Needs you": "Nothing needs you right now.",
  "Working on": "Nothing is in progress right now.",
  Delivered: "No delivered work is saved in this project.",
  Watching: "Nothing is being watched right now.",
};

const NO_RESULT_YET = "No result yet";
const NO_ACTION = "No action to take";
const ACTION_STATUS_UNAVAILABLE = "Action status unavailable";

const opaqueIdentifier = /[0-9a-f]{24,}/i;

function readableSubject(item: WorkItem): string | null {
  const subject = item.subject.resource_id.trim();
  if (subject === "" || opaqueIdentifier.test(subject)) return null;
  return subject;
}

function changedAt(value: string): string | null {
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return null;
  const minutes = Math.round((Date.now() - parsed) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

function changedAtAbsolute(value: string): string | null {
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return null;
  return new Date(parsed).toLocaleString();
}

function WorkRow({
  item,
  endpointIdentity,
  showState,
  selected,
  onOpen,
}: {
  item: WorkItem;
  endpointIdentity: string;
  showState: boolean;
  selected: boolean;
  onOpen: (item: WorkItem, trigger: HTMLButtonElement) => void;
}) {
  const artifactId = item.current_artifact_id;
  const { currentData, isFetching, error } = useGetBuddyDeskActionsQuery(
    {
      id: item.id,
      project_id: item.project.project_id,
      endpoint_identity: endpointIdentity,
    },
    { skip: !artifactId },
  );
  const action = currentData?.data.find(
    (candidate) =>
      candidate.artifact_id === artifactId &&
      candidate.artifact_revision === item.current_artifact_revision &&
      candidate.spec.project.canonical_root === item.project.canonical_root,
  );
  const pending = isFetching && !currentData;
  let verb: string | null = NO_ACTION;
  if (!artifactId) verb = NO_RESULT_YET;
  else if (error) verb = ACTION_STATUS_UNAVAILABLE;
  else if (pending) verb = null;
  else if (action) verb = action.label;
  const changed = changedAt(item.last_material_change_at);
  const absolute = changedAtAbsolute(item.last_material_change_at);
  return (
    <li>
      <button
        className={styles.item}
        aria-current={selected ? "true" : undefined}
        onClick={(event) => onOpen(item, event.currentTarget)}
      >
        <Text size="2" weight="medium" className={styles.name}>
          {item.title}
        </Text>
        <span className={styles.itemMeta}>
          {verb !== null && (
            <Text
              size="1"
              weight={action ? "medium" : "regular"}
              color={action ? "accent" : "gray"}
            >
              {verb}
            </Text>
          )}
          <Text
            size="1"
            color="gray"
            className={showState ? undefined : styles.srOnly}
          >
            {humanizeIdentifier(item.state)}
          </Text>
          {changed && (
            <Text size="1" color="gray" title={absolute ?? undefined}>
              {changed}
              {absolute && (
                <span className={styles.srOnly}>{` · ${absolute}`}</span>
              )}
            </Text>
          )}
        </span>
        <Icon
          icon={ChevronRight}
          size="sm"
          tone="muted"
          className={styles.chevron}
        />
      </button>
    </li>
  );
}

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
  const subject = readableSubject(item);
  useEffect(() => {
    backButton.current?.focus();
  }, []);
  return (
    <article className={styles.detail} aria-label="Work detail">
      <Button
        ref={backButton}
        className={styles.back}
        variant="ghost"
        size="sm"
        leftIcon={ArrowLeft}
        onClick={onBack}
      >
        Back to work list
      </Button>
      <div className={styles.headlineMeta}>
        <Text size="1" color="gray" className={styles.subject}>
          {subject
            ? `${humanizeIdentifier(item.state)} · ${subject}`
            : humanizeIdentifier(item.state)}
        </Text>
      </div>
      {!item.current_artifact_id && (
        <Surface variant="surface-1" className={styles.note} role="status">
          <Text size="2" color="gray">
            {item.title} has not produced a result yet.
          </Text>
        </Surface>
      )}
      {isFetching && (
        <Surface variant="surface-1" className={styles.note} role="status">
          <Text size="2" color="gray">
            Loading the result…
          </Text>
        </Surface>
      )}
      {error && (
        <Surface variant="surface-1" className={styles.note} role="alert">
          <Text size="2" color="red">
            The result could not be loaded.
          </Text>
          <Button size="sm" onClick={() => void refetch()}>
            Retry report
          </Button>
        </Surface>
      )}
      {artifact && !artifactMatches && (
        <Surface variant="surface-1" className={styles.note} role="alert">
          <Text size="2" color="red">
            This result belongs to another project.
          </Text>
        </Surface>
      )}
      {artifact && artifactMatches && !error && (
        <>
          <Surface
            variant="surface-1"
            radius="card"
            className={styles.headline}
          >
            <Text as="p" size="3" weight="bold" className={styles.title}>
              {artifact.title}
            </Text>
            <Text as="p" size="3" className={styles.conclusion}>
              {artifact.conclusion}
            </Text>
            {artifact.delta && (
              <Text as="p" size="2" color="gray" className={styles.delta}>
                {artifact.delta}
              </Text>
            )}
          </Surface>
          <div className={styles.decide}>
            <BuddyDeskActions
              item={item}
              artifact={artifact}
              endpointIdentity={endpointIdentity}
            />
            <div className={styles.embedded}>
              <BuddyReportReadState
                key={`read:${artifact.artifact_id}:${artifact.revision}`}
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
            </div>
          </div>
          <details className={styles.disclosure} open>
            <summary className={styles.summary}>The full write-up</summary>
            <div className={styles.disclosureBody}>
              <BuddyArtifactBody body={artifact.body} />
              {artifact.uncertainty && (
                <section aria-label="Limitations" className={styles.limits}>
                  <Text as="p" size="1" color="orange" weight="medium">
                    What this did not cover
                  </Text>
                  <Text as="p" size="2" color="gray">
                    {artifact.uncertainty}
                  </Text>
                </section>
              )}
            </div>
          </details>
          <details className={styles.disclosure}>
            <summary className={styles.summary}>
              Evidence · revision {artifact.revision}
            </summary>
            <div className={styles.disclosureBody}>
              {artifact.evidence.length === 0 ? (
                <Text as="p" size="2" color="gray">
                  No evidence references were recorded.
                </Text>
              ) : (
                <ul className={styles.evidence}>
                  {artifact.evidence.map((evidence) => (
                    <li key={evidence.evidence_id}>
                      <Text size="2" weight="medium">
                        {humanizeIdentifier(evidence.source.source_kind)}
                      </Text>
                      <Text size="1" color="gray">
                        {evidence.locator}
                      </Text>
                      <Text size="1" color="gray">
                        Observed {evidence.observed_at}
                      </Text>
                    </li>
                  ))}
                </ul>
              )}
            </div>
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
  const [view, setView] = useState<WorkView>(DEFAULT_VIEW);
  const [selectedId, setSelectedId] = useState<string | null>(workId ?? null);
  const [expanded, setExpanded] = useState(false);
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
  const expandButton = useRef<HTMLButtonElement>(null);
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
  const distinctStates = new Set(filtered.map(({ value }) => value.state));
  const filtersApplied = query !== "" || attention !== "all";
  const collapsed =
    !expanded &&
    !workId &&
    !error &&
    !filtersApplied &&
    view === DEFAULT_VIEW &&
    Boolean(currentData) &&
    records.length === 0;

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
  const open = (item: WorkItem, source: HTMLButtonElement) => {
    trigger.current = source;
    setSelectedId(item.id);
    updateLink(item);
  };

  if (collapsed) {
    return (
      <section
        className={styles.collapsed}
        aria-label="Buddy Desk"
        data-detail-open={false}
      >
        <Text size="2" color="gray" className={styles.collapsedText}>
          Nothing needs you right now.
        </Text>
        <Button
          ref={expandButton}
          size="sm"
          variant="ghost"
          leftIcon={Inbox}
          onClick={() => setExpanded(true)}
        >
          Browse saved work
        </Button>
      </section>
    );
  }

  return (
    <Surface
      as="section"
      variant="glass"
      radius="card"
      className={styles.desk}
      aria-label="Buddy Desk"
      data-detail-open={Boolean(selected)}
    >
      <header className={styles.header}>
        <Icon icon={ClipboardList} size="sm" tone="muted" />
        <h2 className={styles.heading}>Buddy Desk</h2>
        <span className={styles.headerSpacer} aria-hidden />
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
        <Surface variant="surface-1" className={styles.note} role="status">
          <Text size="2" color="gray">
            Opening that item…
          </Text>
        </Surface>
      )}
      {selectedId === workId && linkedWork.error && (
        <Surface variant="surface-1" className={styles.note} role="alert">
          <Text size="2" color="red">
            The linked work is unavailable in this project. No other report was
            substituted.
          </Text>
          <div className={styles.noteActions}>
            <Button size="sm" onClick={() => void linkedWork.refetch()}>
              Retry linked work
            </Button>
            <Button size="sm" variant="ghost" onClick={back}>
              Back to work list
            </Button>
          </div>
        </Surface>
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
          {(records.length > 0 || filtersApplied) && !error && (
            <div className={styles.filters}>
              <label className={styles.search}>
                <Text size="1" color="gray">
                  Search saved work
                </Text>
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
                <Text size="1" color="gray">
                  Report attention
                </Text>
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
            </div>
          )}
          {isFetching && (
            <Text
              as="p"
              size="1"
              color="gray"
              role="status"
              className={styles.inlineStatus}
            >
              Loading work…
            </Text>
          )}
          {error ? (
            <Surface variant="surface-1" className={styles.note} role="alert">
              <Text size="2" color="red">
                Buddy could not reach your saved work.
              </Text>
              <Button size="sm" onClick={refresh}>
                Retry work
              </Button>
            </Surface>
          ) : (
            <>
              {currentData && filtered.length === 0 && (
                <Text
                  as="p"
                  size="2"
                  color="gray"
                  className={styles.inlineStatus}
                >
                  {filtersApplied
                    ? "Nothing here matches those filters."
                    : emptyView[view]}
                </Text>
              )}
              <ul className={styles.items}>
                {filtered.map(({ value }) => (
                  <WorkRow
                    key={value.id}
                    item={value}
                    endpointIdentity={endpointIdentity}
                    showState={distinctStates.size > 1}
                    selected={selectedId === value.id}
                    onOpen={open}
                  />
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
          filtered.length > 0 && (
            <div className={styles.placeholder}>
              <Icon icon={FileSearch} size="md" tone="muted" />
              <Text size="2" color="gray">
                Pick something on the left to read what Buddy found.
              </Text>
            </div>
          )
        )}
      </div>
    </Surface>
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
