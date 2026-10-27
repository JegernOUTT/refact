import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import * as Collapsible from "@radix-ui/react-collapsible";
import classNames from "classnames";
import { ChevronDown, Search } from "lucide-react";
import {
  Button,
  ErrorState,
  FieldText,
  Flex,
  Select,
  Spinner,
  Surface,
  Text,
} from "../../../components/ui";
import {
  taskMemoriesApi,
  type TaskMemoryEntry,
  useArchiveTaskMemoryMutation,
  useGetTaskMemoryFacetsQuery,
  useListTaskMemoriesQuery,
  usePinTaskMemoryMutation,
  useTriageTaskMemoriesMutation,
} from "../../../services/refact/taskMemoriesApi";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import { MemoryCard } from "./MemoryCard";
import styles from "./MemoryInboxPanel.module.css";

const ALL_VALUE = "all";

const MEMORY_KINDS = [
  "decision",
  "spec",
  "finding",
  "gotcha",
  "risk",
  "handoff",
  "progress",
  "postmortem",
  "brief",
  "freeform",
] as const;

type MemoryInboxPanelProps = {
  taskId: string;
};

type ActionStatus = "pending" | "success" | "error";
type MemoryActionKind = "pin" | "unpin" | "archive";
type MemoryAction = {
  memory: TaskMemoryEntry;
  kind: MemoryActionKind;
  status: ActionStatus;
};

const ACTION_LABELS = {
  pin: "pinning",
  unpin: "unpinning",
  archive: "archiving",
};

function clientMatches(memory: TaskMemoryEntry, query: string): boolean {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return true;
  return [memory.filename, memory.title, memory.content, memory.namespace]
    .concat(memory.tags)
    .some((value) => value.toLowerCase().includes(normalized));
}

function formatSince(value?: string): string {
  if (!value) return "last cursor";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function useDebouncedValue(value: string, delayMs: number): string {
  const [debounced, setDebounced] = useState(value);

  useEffect(() => {
    const timeout = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(timeout);
  }, [delayMs, value]);

  return debounced;
}

export const MemoryInboxPanel: React.FC<MemoryInboxPanelProps> = ({
  taskId,
}) => {
  const endpoint = useAppSelector((state) =>
    getEngineEndpointIdentity(state.config),
  );
  return (
    <ScopedMemoryInboxPanel
      key={JSON.stringify([endpoint, taskId])}
      taskId={taskId}
    />
  );
};

const ScopedMemoryInboxPanel: React.FC<MemoryInboxPanelProps> = ({
  taskId,
}) => {
  const dispatch = useAppDispatch();
  const [kind, setKind] = useState(ALL_VALUE);
  const [namespace, setNamespace] = useState(ALL_VALUE);
  const [selectedTags, setSelectedTags] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [search, setSearch] = useState("");
  const [tagCloudOpen, setTagCloudOpen] = useState(false);
  const [tagSearch, setTagSearch] = useState("");
  const [expandedMemoryFilename, setExpandedMemoryFilename] = useState<
    string | null
  >(null);
  const [memoryActions, setMemoryActions] = useState<
    ReadonlyMap<string, MemoryAction>
  >(() => new Map());
  const [triageStatus, setTriageStatus] = useState<ActionStatus | null>(null);
  const pendingMemories = useRef(new Set<string>());
  const triagePending = useRef(false);
  const active = useRef(true);
  const debouncedSearch = useDebouncedValue(search, 200);

  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);

  const serverSearch = debouncedSearch.trim();
  const query = useMemo(
    () => ({
      taskId,
      kind: kind === ALL_VALUE ? undefined : kind,
      namespace: namespace === ALL_VALUE ? undefined : namespace,
      search: serverSearch || undefined,
    }),
    [kind, namespace, serverSearch, taskId],
  );
  const { data, isFetching, error, refetch } = useListTaskMemoriesQuery(query);
  const { data: facets } = useGetTaskMemoryFacetsQuery({ taskId });
  const [pinMemory] = usePinTaskMemoryMutation();
  const [archiveMemory] = useArchiveTaskMemoryMutation();
  const [triageDone] = useTriageTaskMemoriesMutation();

  const namespaces = useMemo(
    () => facets?.namespaces ?? [],
    [facets?.namespaces],
  );
  const tags = useMemo(() => facets?.tags ?? [], [facets?.tags]);

  const selectedTagList = useMemo(
    () => [...selectedTags].sort((a, b) => a.localeCompare(b)),
    [selectedTags],
  );

  const filteredTags = useMemo(() => {
    const normalized = tagSearch.trim().toLowerCase();
    if (!normalized) return tags;
    return tags.filter((tag) => tag.toLowerCase().includes(normalized));
  }, [tagSearch, tags]);

  const hasSelectedTags = selectedTagList.length > 0;

  const visibleMemories = useMemo(() => {
    const memories = [...(data?.memories ?? [])];
    for (const action of memoryActions.values()) {
      if (
        action.kind === "archive" &&
        action.status !== "success" &&
        !memories.some((memory) => memory.filename === action.memory.filename)
      ) {
        memories.push(action.memory);
      }
    }
    return memories.filter((memory) => {
      if (kind !== ALL_VALUE && memory.kind !== kind) return false;
      if (namespace !== ALL_VALUE && memory.namespace !== namespace)
        return false;
      if (!clientMatches(memory, search)) return false;
      for (const tag of selectedTags) {
        if (!memory.tags.includes(tag)) return false;
      }
      return true;
    });
  }, [data?.memories, kind, memoryActions, namespace, search, selectedTags]);

  const handleToggleTag = useCallback((tag: string) => {
    setSelectedTags((previous) => {
      const next = new Set(previous);
      if (next.has(tag)) {
        next.delete(tag);
      } else {
        next.add(tag);
      }
      return next;
    });
  }, []);

  const handleClearFilters = useCallback(() => {
    setSelectedTags(new Set());
  }, []);

  const handleExpandedChange = useCallback(
    (filename: string, expanded: boolean) => {
      setExpandedMemoryFilename(expanded ? filename : null);
    },
    [],
  );

  const handleMemoryAction = useCallback(
    async (filename: string, kind: MemoryActionKind) => {
      if (pendingMemories.current.has(filename) || triagePending.current)
        return;
      const memory =
        data?.memories.find((entry) => entry.filename === filename) ??
        memoryActions.get(filename)?.memory;
      if (!memory) return;
      pendingMemories.current.add(filename);
      const action = { memory, kind };
      setMemoryActions((previous) =>
        new Map(previous).set(filename, { ...action, status: "pending" }),
      );
      try {
        if (kind === "archive") {
          const response = await archiveMemory({ taskId, filename }).unwrap();
          if (
            (response.ok as unknown) !== true ||
            response.filename !== filename ||
            !response.archived_filename
          ) {
            throw new Error("Archive response was not confirmed");
          }
        } else {
          const pinned = kind === "pin";
          const response = await pinMemory({
            taskId,
            filename,
            pinned,
          }).unwrap();
          if (
            (response.ok as unknown) !== true ||
            response.filename !== filename ||
            response.pinned !== pinned
          ) {
            throw new Error("Pin response was not confirmed");
          }
        }
        if (!active.current) return;
        setMemoryActions((previous) =>
          new Map(previous).set(filename, { ...action, status: "success" }),
        );
        if (kind === "archive") {
          setExpandedMemoryFilename((current) =>
            current === filename ? null : current,
          );
        }
      } catch {
        if (!active.current) return;
        setMemoryActions((previous) =>
          new Map(previous).set(filename, { ...action, status: "error" }),
        );
      } finally {
        pendingMemories.current.delete(filename);
      }
    },
    [archiveMemory, data?.memories, memoryActions, pinMemory, taskId],
  );

  const handlePin = useCallback(
    (filename: string, pinned: boolean) =>
      handleMemoryAction(filename, pinned ? "pin" : "unpin"),
    [handleMemoryAction],
  );

  const handleArchive = useCallback(
    (filename: string) => handleMemoryAction(filename, "archive"),
    [handleMemoryAction],
  );

  const handleRefresh = useCallback(async () => {
    try {
      await refetch().unwrap();
      if (!active.current) return;
      setMemoryActions(
        (previous) =>
          new Map(
            [...previous].filter(
              ([filename, action]) =>
                action.status !== "error" ||
                action !== memoryActions.get(filename),
            ),
          ),
      );
    } catch {
      return;
    }
  }, [memoryActions, refetch]);

  const handleTriageDone = useCallback(async () => {
    if (triagePending.current) return;
    triagePending.current = true;
    setTriageStatus("pending");
    const cursor = new Date().toISOString();
    const patch = dispatch(
      taskMemoriesApi.util.updateQueryData(
        "listTaskMemories",
        query,
        (draft) => {
          draft.since = cursor;
          draft.new_count = 0;
        },
      ),
    );
    try {
      const response = await triageDone({ taskId, cursor }).unwrap();
      if (
        (response.ok as unknown) !== true ||
        typeof response.cursor !== "string"
      ) {
        throw new Error("Triage response was not confirmed");
      }
      if (!active.current) return;
      setTriageStatus("success");
      dispatch(
        taskMemoriesApi.util.invalidateTags([
          { type: "TaskMemories", id: taskId },
        ]),
      );
    } catch {
      patch.undo();
      if (active.current) setTriageStatus("error");
    } finally {
      triagePending.current = false;
    }
  }, [dispatch, query, taskId, triageDone]);

  return (
    <div className={`${styles.root} rf-enter`}>
      <Flex justify="between" align="start" gap="3" className={styles.header}>
        <div className={styles.headerCopy}>
          <Text weight="bold" size="3" as="div">
            {data?.new_count ?? 0} new since {formatSince(data?.since)}
          </Text>
          <Text size="1" as="div" className={styles.mutedText}>
            {visibleMemories.length} memories shown
            {isFetching ? " · refreshing" : ""}
          </Text>
          <Text size="1" as="div" className={styles.mutedText}>
            Mark all triaged updates the cursor for this entire task, including
            memories hidden by filters. It does not archive memories.
          </Text>
        </div>
        <Button
          size="md"
          variant="soft"
          onClick={() => void handleTriageDone()}
          disabled={triageStatus === "pending"}
          loading={triageStatus === "pending"}
        >
          Mark all triaged
        </Button>
      </Flex>

      <Surface
        animated="rise"
        className={styles.filters}
        radius="card"
        variant="glass"
      >
        <Flex direction="column" gap="2">
          <Flex gap="2" wrap="wrap" align="center" className={styles.filterRow}>
            <Select value={kind} onValueChange={setKind}>
              <Select.Trigger
                aria-label="Memory kind filter"
                className={styles.filterControl}
              />
              <Select.Content>
                <Select.Item value={ALL_VALUE}>All kinds</Select.Item>
                {MEMORY_KINDS.map((item) => (
                  <Select.Item key={item} value={item}>
                    {item}
                  </Select.Item>
                ))}
              </Select.Content>
            </Select>

            <Select value={namespace} onValueChange={setNamespace}>
              <Select.Trigger
                aria-label="Memory namespace filter"
                className={styles.filterControl}
              />
              <Select.Content>
                <Select.Item value={ALL_VALUE}>All namespaces</Select.Item>
                {namespaces.map((item) => (
                  <Select.Item key={item} value={item}>
                    {item}
                  </Select.Item>
                ))}
              </Select.Content>
            </Select>

            <div className={styles.searchBox}>
              <Search aria-hidden="true" className={styles.searchIcon} />
              <FieldText
                value={search}
                onChange={setSearch}
                placeholder="Search memories"
                aria-label="Search memories"
                className={styles.searchInput}
              />
            </div>
          </Flex>

          {(tags.length > 0 || hasSelectedTags) && (
            <Collapsible.Root
              open={tagCloudOpen}
              onOpenChange={setTagCloudOpen}
            >
              <Flex
                align="center"
                justify="between"
                gap="2"
                className={styles.tagSummary}
              >
                <Flex
                  gap="1"
                  wrap="wrap"
                  align="center"
                  className={styles.tagSelectedChips}
                >
                  {selectedTagList.map((tag) => (
                    <button
                      key={tag}
                      type="button"
                      onClick={() => handleToggleTag(tag)}
                      className={classNames(
                        styles.tagChip,
                        styles.tagChipActive,
                        "rf-pressable",
                      )}
                    >
                      {tag}
                    </button>
                  ))}
                  {!hasSelectedTags && (
                    <Text size="1" className={styles.mutedText}>
                      No tag filters selected
                    </Text>
                  )}
                </Flex>
                <Flex align="center" gap="1" className={styles.tagActions}>
                  {hasSelectedTags && (
                    <Button
                      size="sm"
                      variant="plain"
                      onClick={handleClearFilters}
                    >
                      Clear filters
                    </Button>
                  )}
                  <Collapsible.Trigger asChild>
                    <Button size="sm" variant="soft" rightIcon={ChevronDown}>
                      {tagCloudOpen
                        ? "Hide tags"
                        : `Show all ${tags.length} tags`}
                    </Button>
                  </Collapsible.Trigger>
                </Flex>
              </Flex>
              <Collapsible.Content className="rf-expand-grid">
                <div className={styles.tagCloudInner}>
                  <Flex
                    gap="1"
                    wrap="wrap"
                    align="center"
                    className={styles.tagChips}
                  >
                    <div className={styles.tagSearchBox}>
                      <Search
                        aria-hidden="true"
                        className={styles.searchIcon}
                      />
                      <FieldText
                        value={tagSearch}
                        onChange={setTagSearch}
                        placeholder="Filter tags..."
                        aria-label="Filter tags"
                        className={styles.searchInput}
                      />
                    </div>
                    {filteredTags.map((tag) => {
                      const active = selectedTags.has(tag);
                      return (
                        <button
                          key={tag}
                          type="button"
                          onClick={() => handleToggleTag(tag)}
                          className={classNames(
                            styles.tagChip,
                            active && styles.tagChipActive,
                            "rf-pressable",
                          )}
                        >
                          {tag}
                        </button>
                      );
                    })}
                    {filteredTags.length === 0 && (
                      <Text size="1" className={styles.mutedText}>
                        No tags match.
                      </Text>
                    )}
                  </Flex>
                </div>
              </Collapsible.Content>
            </Collapsible.Root>
          )}
        </Flex>
      </Surface>

      {error && (
        <ErrorState
          title="Failed to load task memories."
          variant="compact"
          className={styles.errorState}
        />
      )}

      <Flex direction="column" gap="2" className={`${styles.list} rf-stagger`}>
        {triageStatus === "error" ? (
          <ErrorState
            title="Could not confirm task-wide triage."
            className={styles.errorState}
            description="The previous cursor view was restored. Check your connection and permissions, refresh memories, then use Mark all triaged to retry."
            retry={
              <Button
                size="sm"
                variant="soft"
                onClick={() => void handleRefresh()}
              >
                Refresh memories
              </Button>
            }
          />
        ) : triageStatus ? (
          <Text size="1" role="status" className={styles.actionFeedback}>
            {triageStatus === "pending"
              ? "Marking memories triaged for the entire task… Not yet confirmed."
              : "Server confirmed task-wide triage. Filters did not limit its scope."}
          </Text>
        ) : null}
        {[...memoryActions.values()].map((action) =>
          action.status === "error" ? (
            <ErrorState
              key={action.memory.filename}
              className={styles.errorState}
              title={`Could not confirm ${ACTION_LABELS[action.kind]} “${
                action.memory.filename
              }”.`}
              description={`Check your connection and permissions, then refresh memories to check the server state. ${
                action.kind === "archive"
                  ? "Use Archive and Confirm archive to retry."
                  : `Use ${action.kind === "pin" ? "Pin" : "Unpin"} to retry.`
              }`}
              retry={
                <Button
                  size="sm"
                  variant="soft"
                  onClick={() => void handleRefresh()}
                >
                  Refresh memories
                </Button>
              }
            />
          ) : (
            <Text
              key={action.memory.filename}
              size="1"
              role="status"
              className={styles.actionFeedback}
            >
              {action.status === "pending"
                ? `Request pending: ${ACTION_LABELS[action.kind]} “${
                    action.memory.filename
                  }”. Not yet confirmed.`
                : `Server confirmed ${ACTION_LABELS[action.kind]} “${
                    action.memory.filename
                  }”.`}
            </Text>
          ),
        )}
        {isFetching && !data ? (
          <div className={styles.loadingState}>
            <Spinner />
          </div>
        ) : visibleMemories.length > 0 ? (
          visibleMemories.map((memory) => {
            const pending =
              memoryActions.get(memory.filename)?.status === "pending";
            return (
              <MemoryCard
                key={memory.filename}
                memory={memory}
                onPin={handlePin}
                onArchive={handleArchive}
                disabled={triageStatus === "pending" || pending}
                pending={pending}
                expanded={expandedMemoryFilename === memory.filename}
                onExpandedChange={handleExpandedChange}
              />
            );
          })
        ) : (
          <Text as="div" className={styles.emptyState}>
            No memories match the current filters.
          </Text>
        )}
      </Flex>
    </div>
  );
};

export default MemoryInboxPanel;
