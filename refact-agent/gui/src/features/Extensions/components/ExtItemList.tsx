import React from "react";
import { Plus, Trash2 } from "lucide-react";
import { Badge, Button, EmptyState, IconButton } from "../../../components/ui";
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
      {items.map((item) => (
        <div
          key={item.name}
          className={`${styles.item} ${
            selectedId === item.name ? styles.selected : ""
          }`}
        >
          <button
            type="button"
            aria-label={`Select ${item.name}`}
            aria-current={selectedId === item.name ? "true" : undefined}
            className={`${styles.main} rf-pressable ${
              selectedId === item.name ? styles.selected : ""
            }`}
            onClick={() => onSelect(item.name)}
          >
            <span className={styles.content}>
              <span className={styles.title}>{item.name}</span>
              <span className={styles.description}>{item.description}</span>
            </span>
          </button>
          <span className={styles.meta}>
            <Badge tone="muted">{SCOPE_LABELS[item.scope]}</Badge>
            {item.read_only ? (
              <span aria-hidden="true" className={styles.deletePlaceholder} />
            ) : (
              <IconButton
                variant="danger"
                size="sm"
                icon={Trash2}
                aria-label={`Delete ${item.name}`}
                onClick={() => onDelete(item.name, item.scope)}
              />
            )}
          </span>
        </div>
      ))}
      {items.length === 0 && (
        <EmptyState
          title="No items found"
          description="Create one to get started."
        />
      )}
    </div>
  );
};
