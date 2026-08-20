import React from "react";
import { Plus, Trash2 } from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  IconButton,
  VirtualizedGrid,
} from "../../../components/ui";
import type {
  SkillRegistryItem,
  CommandRegistryItem,
} from "../../../services/refact/extensions";
import styles from "./ExtItemList.module.css";

export type RegistryItem = SkillRegistryItem | CommandRegistryItem;

type ExtItemListProps = {
  items: RegistryItem[];
  selectedId: string | null;
  onSelect: (name: string) => void;
  onCreate: () => void;
  onDelete: (name: string, scope: "global" | "local" | "plugin") => void;
};

const SCOPE_LABELS = {
  global: "Global",
  local: "Local",
  plugin: "Plugin",
} as const;

const SCOPE_TONES = {
  global: "accent",
  local: "success",
  plugin: "muted",
} as const;

const ITEM_ROW_GAP = 4;

export const ExtItemList: React.FC<ExtItemListProps> = ({
  items,
  selectedId,
  onSelect,
  onCreate,
  onDelete,
}) => {
  return (
    <div className={`${styles.list} rf-stagger`}>
      <Button
        variant="soft"
        onClick={onCreate}
        size="sm"
        leftIcon={Plus}
        className={styles.createButton}
      >
        New
      </Button>
      {items.length === 0 ? (
        <EmptyState
          title="No items found"
          description="Create one to get started."
        />
      ) : (
        <VirtualizedGrid
          items={items}
          columns={1}
          gap={ITEM_ROW_GAP}
          getItemKey={(item) => item.name}
          aria-label="Extensions"
          renderItem={(item) => (
            <div
              role="button"
              tabIndex={0}
              aria-label={`Select ${item.name}`}
              aria-current={selectedId === item.name ? "true" : undefined}
              className={`${styles.itemRow} rf-pressable ${
                selectedId === item.name ? styles.selected : ""
              }`}
              onClick={() => onSelect(item.name)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onSelect(item.name);
                }
              }}
            >
              <div className={styles.rowInfo}>
                <span className={styles.rowTitle}>{item.name}</span>
                <span className={styles.rowDescription}>
                  {item.description}
                </span>
              </div>
              <Badge
                className={styles.scopeBadge}
                tone={SCOPE_TONES[item.scope]}
              >
                {SCOPE_LABELS[item.scope]}
              </Badge>
              {item.read_only ? (
                <span aria-hidden="true" className={styles.deletePlaceholder} />
              ) : (
                <IconButton
                  variant="ghost"
                  size="sm"
                  icon={Trash2}
                  className={styles.deleteBtn}
                  aria-label={`Delete ${item.name}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    onDelete(item.name, item.scope);
                  }}
                />
              )}
            </div>
          )}
        />
      )}
    </div>
  );
};
