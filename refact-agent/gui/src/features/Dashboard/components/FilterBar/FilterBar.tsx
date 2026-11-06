import React, { useCallback, useEffect, useRef, useState } from "react";
import { Search } from "lucide-react";
import { Icon, SegmentedControl } from "../../../../components/ui";
import { useAppSelector } from "../../../../hooks";
import { selectStreamItems, type StreamFilter } from "../../streamSelectors";
import styles from "./FilterBar.module.css";

const SEARCH_DEBOUNCE_MS = 200;

type FilterBarProps = {
  filter: StreamFilter;
  onFilterChange: (filter: StreamFilter) => void;
};

export const FilterBar: React.FC<FilterBarProps> = ({
  filter,
  onFilterChange,
}) => {
  const items = useAppSelector(selectStreamItems);

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
    </div>
  );
};
