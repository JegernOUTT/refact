import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
  Badge,
  Box,
  Button,
  Callout,
  Flex,
  Select,
  Spinner,
  Text,
  TextField,
} from "@radix-ui/themes";
import {
  ExclamationTriangleIcon,
  MagnifyingGlassIcon,
} from "@radix-ui/react-icons";
import classNames from "classnames";
import {
  taskMemoriesApi,
  type TaskMemoryEntry,
  useArchiveTaskMemoryMutation,
  useGetTaskMemoryFacetsQuery,
  useListTaskMemoriesQuery,
  usePinTaskMemoryMutation,
  useTriageTaskMemoriesMutation,
} from "../../../services/refact/taskMemoriesApi";
import { useAppDispatch } from "../../../hooks";
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

function optimisticKey(taskId: string, filename: string): string {
  return `${taskId}:${filename}`;
}

export const MemoryInboxPanel: React.FC<MemoryInboxPanelProps> = ({
  taskId,
}) => {
  const dispatch = useAppDispatch();
  const [kind, setKind] = useState(ALL_VALUE);
  const [namespace, setNamespace] = useState(ALL_VALUE);
  const [selectedTags, setSelectedTags] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [search, setSearch] = useState("");
  const [optimisticPinned, setOptimisticPinned] = useState<
    ReadonlyMap<string, boolean>
  >(() => new Map());
  const [archived, setArchived] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [pendingMemoryKeys, setPendingMemoryKeys] = useState<
    ReadonlySet<string>
  >(() => new Set());
  const debouncedSearch = useDebouncedValue(search, 200);

  useEffect(() => {
    setOptimisticPinned(new Map());
    setArchived(new Set());
    setPendingMemoryKeys(new Set());
  }, [taskId]);

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
  const { data, isFetching, error } = useListTaskMemoriesQuery(query);
  const { data: facets } = useGetTaskMemoryFacetsQuery({ taskId });
  const [pinMemory] = usePinTaskMemoryMutation();
  const [archiveMemory] = useArchiveTaskMemoryMutation();
  const [triageDone, triageState] = useTriageTaskMemoriesMutation();

  const memoriesWithOptimisticState = useMemo(() => {
    return (data?.memories ?? [])
      .filter((memory) => !archived.has(optimisticKey(taskId, memory.filename)))
      .map((memory) => ({
        ...memory,
        pinned:
          optimisticPinned.get(optimisticKey(taskId, memory.filename)) ??
          memory.pinned,
      }));
  }, [archived, data?.memories, optimisticPinned, taskId]);

  const namespaces = useMemo(
    () => facets?.namespaces ?? [],
    [facets?.namespaces],
  );
  const tags = useMemo(() => facets?.tags ?? [], [facets?.tags]);

  const selectedTagList = useMemo(
    () => [...selectedTags].sort((a, b) => a.localeCompare(b)),
    [selectedTags],
  );

  const staleSelectedTags = useMemo(() => {
    const currentTags = new Set(tags);
    return selectedTagList.filter((tag) => !currentTags.has(tag));
  }, [selectedTagList, tags]);

  const hasSelectedTags = selectedTagList.length > 0;

  const visibleMemories = useMemo(() => {
    return memoriesWithOptimisticState.filter((memory) => {
      if (!clientMatches(memory, search)) return false;
      for (const tag of selectedTags) {
        if (!memory.tags.includes(tag)) return false;
      }
      return true;
    });
  }, [memoriesWithOptimisticState, search, selectedTags]);

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

  const handlePin = useCallback(
    async (filename: string, pinned: boolean) => {
      const key = optimisticKey(taskId, filename);
      setPendingMemoryKeys((previous) => new Set(previous).add(key));
      setOptimisticPinned((previous) => new Map(previous).set(key, pinned));
      try {
        await pinMemory({ taskId, filename, pinned }).unwrap();
      } catch {
        setOptimisticPinned((previous) => new Map(previous).set(key, !pinned));
      } finally {
        setPendingMemoryKeys((previous) => {
          const next = new Set(previous);
          next.delete(key);
          return next;
        });
      }
    },
    [pinMemory, taskId],
  );

  const handleArchive = useCallback(
    async (filename: string) => {
      const key = optimisticKey(taskId, filename);
      setPendingMemoryKeys((previous) => new Set(previous).add(key));
      setArchived((previous) => new Set(previous).add(key));
      try {
        await archiveMemory({ taskId, filename }).unwrap();
      } catch {
        setArchived((previous) => {
          const next = new Set(previous);
          next.delete(key);
          return next;
        });
      } finally {
        setPendingMemoryKeys((previous) => {
          const next = new Set(previous);
          next.delete(key);
          return next;
        });
      }
    },
    [archiveMemory, taskId],
  );

  const handleTriageDone = useCallback(async () => {
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
      await triageDone({ taskId, cursor }).unwrap();
      dispatch(
        taskMemoriesApi.util.invalidateTags([
          { type: "TaskMemories", id: taskId },
        ]),
      );
    } catch {
      patch.undo();
    }
  }, [dispatch, query, taskId, triageDone]);

  return (
    <Box className={styles.root}>
      <Flex justify="between" align="start" gap="3" className={styles.header}>
        <Box>
          <Text weight="bold" size="3" as="div">
            {data?.new_count ?? 0} new since {formatSince(data?.since)}
          </Text>
          <Text size="1" color="gray" as="div">
            {visibleMemories.length} memories shown
            {isFetching ? " · refreshing" : ""}
          </Text>
        </Box>
        <Button
          size="2"
          variant="soft"
          onClick={() => void handleTriageDone()}
          disabled={triageState.isLoading}
        >
          {triageState.isLoading ? <Spinner size="1" /> : "Mark all triaged"}
        </Button>
      </Flex>

      <Flex direction="column" gap="2" className={styles.filters}>
        <Flex gap="2" wrap="wrap" align="center">
          <Select.Root value={kind} onValueChange={setKind} size="1">
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
          </Select.Root>

          <Select.Root value={namespace} onValueChange={setNamespace} size="1">
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
          </Select.Root>

          <Box className={styles.searchBox}>
            <TextField.Root
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search memories"
              aria-label="Search memories"
            >
              <TextField.Slot>
                <MagnifyingGlassIcon />
              </TextField.Slot>
            </TextField.Root>
          </Box>
        </Flex>

        {(tags.length > 0 || hasSelectedTags) && (
          <Flex gap="1" wrap="wrap" align="center" className={styles.tagChips}>
            {tags.map((tag) => {
              const active = selectedTags.has(tag);
              return (
                <Badge
                  key={tag}
                  asChild
                  color={active ? "blue" : "gray"}
                  variant={active ? "solid" : "outline"}
                  className={classNames(
                    styles.tagChip,
                    active && styles.tagChipActive,
                  )}
                >
                  <button type="button" onClick={() => handleToggleTag(tag)}>
                    {tag}
                  </button>
                </Badge>
              );
            })}
            {staleSelectedTags.map((tag) => (
              <Badge
                key={tag}
                asChild
                color="gray"
                variant="outline"
                className={classNames(styles.tagChip, styles.tagChipStale)}
              >
                <button type="button" onClick={() => handleToggleTag(tag)}>
                  {tag}
                </button>
              </Badge>
            ))}
            {hasSelectedTags && (
              <Button size="1" variant="ghost" onClick={handleClearFilters}>
                Clear filters
              </Button>
            )}
          </Flex>
        )}
      </Flex>

      {error && (
        <Callout.Root color="red" size="1">
          <Callout.Icon>
            <ExclamationTriangleIcon />
          </Callout.Icon>
          <Callout.Text>Failed to load task memories.</Callout.Text>
        </Callout.Root>
      )}

      <Flex direction="column" gap="2" className={styles.list}>
        {isFetching && !data ? (
          <Flex justify="center" p="4">
            <Spinner />
          </Flex>
        ) : visibleMemories.length > 0 ? (
          visibleMemories.map((memory) => {
            const pending = pendingMemoryKeys.has(
              optimisticKey(taskId, memory.filename),
            );
            return (
              <MemoryCard
                key={memory.filename}
                memory={memory}
                onPin={handlePin}
                onArchive={handleArchive}
                disabled={triageState.isLoading || pending}
                pending={pending}
              />
            );
          })
        ) : (
          <Text color="gray" size="2" className={styles.emptyState}>
            No memories match the current filters.
          </Text>
        )}
      </Flex>
    </Box>
  );
};

export default MemoryInboxPanel;
