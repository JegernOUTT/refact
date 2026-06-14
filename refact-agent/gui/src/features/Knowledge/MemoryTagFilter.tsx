import { useMemo, useState } from "react";
import { Check, Tags, X } from "lucide-react";

import { Badge, Button, Icon, Popover } from "../../components/ui";
import styles from "./MemoryTagFilter.module.css";

const MAX_RENDERED_TAGS = 200;

interface MemoryTagFilterProps {
  allTags: string[];
  selectedTags: Set<string>;
  onToggleTag: (tag: string) => void;
  onClearTags: () => void;
}

export function MemoryTagFilter({
  allTags,
  selectedTags,
  onToggleTag,
  onClearTags,
}: MemoryTagFilterProps) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");

  const matches = useMemo(() => {
    const query = search.trim().toLowerCase();
    const base = query
      ? allTags.filter((tag) => tag.toLowerCase().includes(query))
      : allTags;
    return base.slice(0, MAX_RENDERED_TAGS);
  }, [allTags, search]);

  const selectedList = useMemo(() => [...selectedTags].sort(), [selectedTags]);

  if (allTags.length === 0) return null;

  return (
    <div className={styles.filter}>
      <Popover open={open} onOpenChange={setOpen}>
        <Popover.Trigger asChild>
          <Button variant="outline" size="sm" className={styles.trigger}>
            <Icon icon={Tags} size="sm" />
            Filter by tag
            {selectedTags.size > 0 ? (
              <Badge tone="accent">{selectedTags.size}</Badge>
            ) : null}
          </Button>
        </Popover.Trigger>
        <Popover.Content
          className={styles.popover}
          align="start"
          maxWidth="320px"
          maxHeight="360px"
          scrollable={false}
        >
          <div className={styles.searchRow}>
            <input
              className={styles.searchInput}
              type="text"
              placeholder="Search tags..."
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              aria-label="Search tags"
              autoFocus
            />
          </div>
          <div className={styles.list}>
            {matches.length === 0 ? (
              <p className={styles.empty}>No matching tags</p>
            ) : (
              matches.map((tag) => {
                const isSelected = selectedTags.has(tag);
                return (
                  <button
                    key={tag}
                    type="button"
                    className={styles.option}
                    onClick={() => onToggleTag(tag)}
                    aria-pressed={isSelected}
                  >
                    <span className={styles.optionLabel}>{tag}</span>
                    {isSelected ? (
                      <Icon icon={Check} size="sm" tone="accent" />
                    ) : null}
                  </button>
                );
              })
            )}
          </div>
        </Popover.Content>
      </Popover>

      {selectedList.length > 0 ? (
        <div className={styles.selected}>
          {selectedList.map((tag) => (
            <button
              key={tag}
              type="button"
              className={styles.chip}
              onClick={() => onToggleTag(tag)}
              aria-label={`Remove ${tag} filter`}
            >
              <span className={styles.chipLabel}>{tag}</span>
              <Icon icon={X} size="sm" />
            </button>
          ))}
          <button type="button" className={styles.clear} onClick={onClearTags}>
            Clear
          </button>
        </div>
      ) : null}
    </div>
  );
}
