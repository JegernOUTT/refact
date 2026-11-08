import React, { useEffect, useRef, useState } from "react";
import { humanizeIdentifier } from "../../../../utils/displayNames";
import { Search } from "lucide-react";
import { Button, Icon, Popover, Select } from "../../../../components/ui";
import { useAppSelector } from "../../../../hooks/useAppSelector";
import { selectStreamItems } from "../../streamSelectors";
import type { StreamFilter } from "../../streamSelectors";
import { useOrganizationData } from "../../hooks/useOrganization";
import styles from "./FilterBar.module.css";

type FilterBarProps = {
  filter: StreamFilter;
  onFilterChange: (filter: StreamFilter) => void;
};
export const FilterBar: React.FC<FilterBarProps> = ({
  filter,
  onFilterChange,
}) => {
  const organization = useOrganizationData();
  const items = useAppSelector(selectStreamItems);
  const topics = (organization.topics.currentData?.items ?? []).filter(
    (topic) => !topic.archived,
  );
  const tags = (organization.tags.currentData?.items ?? []).filter(
    (tag) => !tag.archived,
  );
  const toolbarRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(1000);
  useEffect(() => {
    const element = toolbarRef.current;
    if (!element) return undefined;
    const observer = new ResizeObserver(([entry]) =>
      setWidth(entry.contentRect.width),
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const compact = width < 700;
  const minimal = width < 400;
  const [queryDraft, setQueryDraft] = useState(filter.query);
  useEffect(() => setQueryDraft(filter.query), [filter.query]);
  const filterRef = useRef(filter);
  filterRef.current = filter;
  const changeRef = useRef(onFilterChange);
  changeRef.current = onFilterChange;
  useEffect(() => {
    if (queryDraft === filterRef.current.query) return undefined;
    const timer = setTimeout(
      () => changeRef.current({ ...filterRef.current, query: queryDraft }),
      200,
    );
    return () => clearTimeout(timer);
  }, [queryDraft]);
  const select = (
    label: string,
    value: string,
    options: { value: string; label: string }[],
    change: (value: string) => void,
  ) => (
    <Select value={value} onValueChange={change}>
      <Select.Trigger aria-label={label} className={styles.select} />
      <Select.Content>
        {options.map((option) => (
          <Select.Item key={option.value} value={option.value}>
            {option.label}
          </Select.Item>
        ))}
      </Select.Content>
    </Select>
  );
  const controls = (
    <>
      {select(
        "Type",
        filter.kind,
        [
          { value: "all", label: `All types (${items.length})` },
          {
            value: "chat",
            label: `Chats (${
              items.filter((item) => item.kind === "chat").length
            })`,
          },
          {
            value: "task",
            label: `Tasks (${
              items.filter((item) => item.kind === "task").length
            })`,
          },
        ],
        (kind) => {
          if (kind === "all" || kind === "chat" || kind === "task")
            onFilterChange({ ...filter, kind });
        },
      )}
      {select(
        "Status",
        filter.status ?? "all",
        [
          "all",
          "streaming",
          "working",
          "idle",
          "done",
          "failed",
          "paused",
          "planning",
        ].map((value) => ({
          value,
          label: value === "all" ? "All statuses" : humanizeIdentifier(value),
        })),
        (status) => onFilterChange({ ...filter, status }),
      )}
      {select(
        "Sort",
        filter.sort ?? "recent",
        [
          { value: "recent", label: "Recent" },
          { value: "oldest", label: "Oldest" },
          { value: "title", label: "Title" },
        ],
        (sort) => onFilterChange({ ...filter, sort }),
      )}
    </>
  );
  const taxonomyControls = (
    <>
      {topics.length > 0 &&
        select(
          "Topics",
          filter.topicId ?? "all",
          [
            { value: "all", label: "Topics" },
            { value: "unclassified", label: "No topic" },
            ...(organization.topics.currentData?.items ?? [])
              .filter((topic) => !topic.archived)
              .map((topic) => ({ value: topic.id, label: topic.name })),
          ],
          (topicId) => onFilterChange({ ...filter, topicId }),
        )}
      {tags.length > 0 && (
        <Popover>
          <Popover.Trigger asChild>
            <Button size="sm" variant="ghost">
              Tags{filter.tagIds?.length ? ` (${filter.tagIds.length})` : ""}
            </Button>
          </Popover.Trigger>
          <Popover.Content>
            <div className={styles.options}>
              {(organization.tags.currentData?.items ?? [])
                .filter((tag) => !tag.archived)
                .map((tag) => (
                  <label key={tag.id}>
                    <input
                      type="checkbox"
                      checked={filter.tagIds?.includes(tag.id) ?? false}
                      onChange={(event) =>
                        onFilterChange({
                          ...filter,
                          tagIds: event.target.checked
                            ? [...(filter.tagIds ?? []), tag.id]
                            : filter.tagIds?.filter((id) => id !== tag.id),
                        })
                      }
                    />
                    {tag.name}
                  </label>
                ))}
              {!organization.tags.currentData?.items.length && (
                <span>No tags yet</span>
              )}
            </div>
          </Popover.Content>
        </Popover>
      )}
    </>
  );
  return (
    <div ref={toolbarRef} className={styles.filterBar}>
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
      {!minimal && taxonomyControls}
      {!compact && <div className={styles.secondary}>{controls}</div>}
      {compact && (
        <div className={styles.overflow}>
          <Popover>
            <Popover.Trigger asChild>
              <Button size="sm" variant="ghost" aria-label="More filters">
                Filters
              </Button>
            </Popover.Trigger>
            <Popover.Content>
              <div className={styles.options}>
                {minimal && taxonomyControls}
                {controls}
              </div>
            </Popover.Content>
          </Popover>
        </div>
      )}
    </div>
  );
};
