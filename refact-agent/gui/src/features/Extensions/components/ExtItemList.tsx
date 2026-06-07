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
      <Button variant="soft" onClick={onCreate} size="sm" leftIcon={Plus}>
        New
      </Button>
      {items.map((item) => (
        <button
          key={item.name}
          type="button"
          aria-label={`Select ${item.name}`}
          className={`${styles.item} rf-pressable ${
            selectedId === item.name ? styles.selected : ""
          }`}
          onClick={() => onSelect(item.name)}
        >
          <span className={styles.content}>
            <span className={styles.title}>{item.name}</span>
            <span className={styles.description}>{item.description}</span>
          </span>
          <span className={styles.meta}>
            <Badge tone="muted">{SCOPE_LABELS[item.scope]}</Badge>
            {!item.read_only && (
              <IconButton
                variant="danger"
                size="sm"
                icon={Trash2}
                aria-label={`Delete `}
                onClick={(e) => {
                  e.stopPropagation();
                  onDelete(item.name, item.scope);
                }}
              />
            )}
          </span>
        </button>
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
