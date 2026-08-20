import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  ListPlus,
  MessageSquarePlus,
  Plus,
  Search,
} from "lucide-react";
import {
  Button,
  Icon,
  Menu,
  SegmentedControl,
} from "../../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../../hooks";
import { newChatAction } from "../../../Chat/Thread";
import { push } from "../../../Pages/pagesSlice";
import { useCreateTaskMutation } from "../../../../services/refact/tasks";
import {
  selectAttentionItems,
  selectStreamItems,
  selectTodayAggregate,
  type StreamFilter,
} from "../../streamSelectors";
import styles from "./FilterBar.module.css";

const SEARCH_DEBOUNCE_MS = 200;

type FilterBarProps = {
  filter: StreamFilter;
  attentionActive: boolean;
  onFilterChange: (filter: StreamFilter) => void;
  onToggleAttention: () => void;
};

export const FilterBar: React.FC<FilterBarProps> = ({
  filter,
  attentionActive,
  onFilterChange,
  onToggleAttention,
}) => {
  const dispatch = useAppDispatch();
  const items = useAppSelector(selectStreamItems);
  const attentionItems = useAppSelector(selectAttentionItems);
  const aggregate = useAppSelector(selectTodayAggregate);
  const [createTask, { isLoading: isCreatingTask }] = useCreateTaskMutation();

  const [queryDraft, setQueryDraft] = useState(filter.query);
  const filterRef = useRef(filter);
  filterRef.current = filter;
  const onFilterChangeRef = useRef(onFilterChange);
  onFilterChangeRef.current = onFilterChange;

  useEffect(() => {
    if (queryDraft === filterRef.current.query) return undefined;
    const timer = setTimeout(() => {
      onFilterChangeRef.current({
        ...filterRef.current,
        query: queryDraft,
      });
    }, SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [queryDraft]);

  const chatCount = items.filter((item) => item.kind === "chat").length;
  const taskCount = items.filter((item) => item.kind === "task").length;

  const handleKindChange = useCallback(
    (value: string) => {
      if (value !== "all" && value !== "chat" && value !== "task") return;
      onFilterChange({ ...filterRef.current, kind: value });
    },
    [onFilterChange],
  );

  const handleNewChat = useCallback(() => {
    dispatch(newChatAction());
    dispatch(push({ name: "chat" }));
  }, [dispatch]);

  const handleNewTask = useCallback(() => {
    void createTask({ name: "New Task" })
      .unwrap()
      .then((task) => {
        dispatch(push({ name: "task workspace", taskId: task.id }));
      })
      .catch(() => undefined);
  }, [createTask, dispatch]);

  const aggregateText = `today · ${
    aggregate.chats
  } chats · $${aggregate.costUsd.toFixed(2)}`;

  return (
    <div className={styles.filterBar}>
      <SegmentedControl
        size="sm"
        className={styles.segmented}
        value={filter.kind}
        onValueChange={handleKindChange}
        options={[
          { value: "all", label: "All" },
          { value: "chat", label: `Chats ${chatCount}` },
          { value: "task", label: `Tasks ${taskCount}` },
        ]}
      />

      {attentionItems.length > 0 ? (
        <button
          type="button"
          className={`${styles.attentionChip} rf-pressable`}
          onClick={onToggleAttention}
          aria-pressed={attentionActive}
          data-active={attentionActive || undefined}
        >
          <Icon icon={AlertTriangle} size="sm" tone="warning" />
          {attentionItems.length}
        </button>
      ) : null}

      <label className={styles.searchField}>
        <Icon icon={Search} size="sm" tone="muted" />
        <input
          type="search"
          className={styles.searchInput}
          placeholder="Search…"
          aria-label="Search chats and tasks"
          value={queryDraft}
          onChange={(event) => setQueryDraft(event.target.value)}
        />
      </label>

      <span className={styles.aggregate}>{aggregateText}</span>

      <Menu>
        <Menu.Trigger asChild>
          <Button
            variant="soft"
            size="sm"
            leftIcon={Plus}
            loading={isCreatingTask}
            className={styles.newButton}
          >
            New
          </Button>
        </Menu.Trigger>
        <Menu.Content align="end">
          <Menu.Item onSelect={handleNewChat}>
            <Icon icon={MessageSquarePlus} size="sm" tone="muted" />
            New Chat
          </Menu.Item>
          <Menu.Item onSelect={handleNewTask}>
            <Icon icon={ListPlus} size="sm" tone="muted" />
            New Task
          </Menu.Item>
        </Menu.Content>
      </Menu>
    </div>
  );
};
