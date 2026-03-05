import React from "react";
import { Flex, Button, Text, Badge, IconButton } from "@radix-ui/themes";
import { PlusIcon, TrashIcon } from "@radix-ui/react-icons";
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
    <Flex direction="column" gap="1" className={styles.list}>
      <Button variant="soft" onClick={onCreate} size="1">
        <PlusIcon /> New
      </Button>
      {items.map((item) => (
        <div
          key={item.name}
          role="button"
          tabIndex={0}
          aria-label={`Select ${item.name}`}
          className={`${styles.item} ${
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
          <Flex direction="column" gap="0" style={{ minWidth: 0, flex: 1 }}>
            <Text
              size="1"
              weight="medium"
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {item.name}
            </Text>
            <Text
              size="1"
              color="gray"
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {item.description}
            </Text>
          </Flex>
          <Flex gap="1" align="center" style={{ flexShrink: 0 }}>
            <Badge size="1" variant="soft">
              {SCOPE_LABELS[item.scope]}
            </Badge>
            {!item.read_only && (
              <IconButton
                size="1"
                variant="ghost"
                color="red"
                aria-label={`Delete ${item.name}`}
                onClick={(e) => {
                  e.stopPropagation();
                  onDelete(item.name, item.scope);
                }}
              >
                <TrashIcon />
              </IconButton>
            )}
          </Flex>
        </div>
      ))}
      {items.length === 0 && (
        <Text size="1" color="gray">
          No items found
        </Text>
      )}
    </Flex>
  );
};
