import React from "react";
import { Check, ChevronDown, Plus, Search, X } from "lucide-react";

import { Badge } from "../Badge";
import { Button } from "../Button";
import { Icon } from "../Icon";
import { Popover } from "../Popover";
import styles from "./ModelSelector.module.css";

export type ModelSelectorBadge =
  | "default"
  | "reasoning"
  | "light"
  | "buddy"
  | "task-agent"
  | "chat2";

export interface ModelOption {
  value: string;
  displayName: string;
  group?: string;
  disabled?: boolean;
  pricing?: {
    prompt: string;
    output: string;
  };
  contextWindow?: string;
  badges?: ModelSelectorBadge[];
  capabilities?: React.ReactNode;
}

export interface ModelSelectorGroup {
  id: string;
  label: string;
}

export interface ModelSelectorProps {
  models: ModelOption[];
  value: string | null;
  onSelect: (value: string) => void;
  groups?: ModelSelectorGroup[];
  allowUnset?: boolean;
  unsetLabel?: string;
  disabled?: boolean;
  onAddNewModel?: () => void;
  triggerSize?: React.ComponentProps<typeof Button>["size"];
  variant?: "popover" | "inline";
}

const unsetValue = "__refact_model_selector_unset__";

const badgeLabel: Record<ModelSelectorBadge, string> = {
  buddy: "Companion",
  chat2: "Chat 2",
  default: "Default",
  light: "Light",
  reasoning: "Reasoning",
  "task-agent": "Task Agent",
};

const badgeTone: Record<
  ModelSelectorBadge,
  React.ComponentProps<typeof Badge>["tone"]
> = {
  buddy: "warning",
  chat2: "accent",
  default: "accent",
  light: "success",
  reasoning: "default",
  "task-agent": "accent",
};

type ModelSelectorListPropsBase = Omit<
  ModelSelectorProps,
  "triggerSize" | "variant"
>;

interface ModelSelectorListProps extends ModelSelectorListPropsBase {
  onRequestClose?: () => void;
}

interface RenderableGroup {
  id: string;
  label: string;
  models: ModelOption[];
}

function normalize(text: string) {
  return text.trim().toLocaleLowerCase();
}

function getCurrentLabel(
  models: ModelOption[],
  value: string | null,
  allowUnset?: boolean,
  unsetLabel = "No model selected",
) {
  if (value === null && allowUnset) {
    return unsetLabel;
  }

  return (
    models.find((model) => model.value === value)?.displayName ??
    value ??
    "Select model"
  );
}

function buildGroups(models: ModelOption[], groups?: ModelSelectorGroup[]) {
  const groupOrder = groups ?? [];
  const knownGroups = new Set(groupOrder.map((group) => group.id));
  const rendered: RenderableGroup[] = groupOrder.map((group) => ({
    ...group,
    models: models.filter((model) => model.group === group.id),
  }));
  const ungrouped = models.filter(
    (model) => !model.group || !knownGroups.has(model.group),
  );

  if (ungrouped.length > 0) {
    rendered.push({
      id: "__ungrouped__",
      label: groupOrder.length > 0 ? "Other models" : "Models",
      models: ungrouped,
    });
  }

  return rendered.filter((group) => group.models.length > 0);
}

function ModelSelectorList({
  allowUnset,
  disabled,
  groups,
  models,
  onAddNewModel,
  onRequestClose,
  onSelect,
  unsetLabel,
  value,
}: ModelSelectorListProps) {
  const emptyLabel = unsetLabel ?? "No model selected";
  const [query, setQuery] = React.useState("");
  const selectedRef = React.useRef<HTMLButtonElement>(null);
  const searchInputRef = React.useRef<HTMLInputElement | null>(null);
  const normalizedQuery = normalize(query);
  const filteredModels = React.useMemo(() => {
    if (!normalizedQuery) {
      return models;
    }

    return models.filter((model) => {
      const haystack = [
        model.displayName,
        model.value,
        model.group ?? "",
        model.contextWindow ?? "",
        model.pricing?.prompt ?? "",
        model.pricing?.output ?? "",
        ...(model.badges ?? []),
      ]
        .join(" ")
        .toLocaleLowerCase();

      return haystack.includes(normalizedQuery);
    });
  }, [models, normalizedQuery]);
  const renderedGroups = React.useMemo(
    () => buildGroups(filteredModels, groups),
    [filteredModels, groups],
  );

  React.useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, [renderedGroups, value]);

  const handleSelect = React.useCallback(
    (nextValue: string) => {
      onSelect(nextValue);
      onRequestClose?.();
    },
    [onRequestClose, onSelect],
  );

  return (
    <div className={styles.listRoot}>
      <label className={styles.searchBox}>
        <Icon icon={Search} size="sm" tone="muted" />
        <input
          ref={searchInputRef}
          className={styles.searchInput}
          placeholder="Search models"
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>

      <div className={styles.scrollArea} role="listbox" aria-label="Models">
        {allowUnset ? (
          <ModelRow
            model={{ value: unsetValue, displayName: emptyLabel }}
            selected={value === null}
            selectedRef={value === null ? selectedRef : undefined}
            onSelect={() => handleSelect("")}
          />
        ) : null}

        {renderedGroups.length > 0 ? (
          renderedGroups.map((group) => (
            <div key={group.id} className={styles.group}>
              <div className={styles.groupLabel}>{group.label}</div>
              <div className={styles.groupItems}>
                {group.models.map((model) => (
                  <ModelRow
                    key={model.value}
                    model={model}
                    selected={model.value === value}
                    selectedRef={
                      model.value === value ? selectedRef : undefined
                    }
                    onSelect={() => handleSelect(model.value)}
                  />
                ))}
              </div>
            </div>
          ))
        ) : (
          <div className={styles.empty}>No models match your search.</div>
        )}
      </div>

      {onAddNewModel ? (
        <button
          className={styles.addButton}
          disabled={disabled}
          type="button"
          onClick={() => {
            onAddNewModel();
            onRequestClose?.();
          }}
        >
          <Icon icon={Plus} size="sm" tone="accent" />
          <span>Add new model...</span>
        </button>
      ) : null}
    </div>
  );
}

interface ModelRowProps {
  model: ModelOption;
  selected: boolean;
  selectedRef?: React.RefObject<HTMLButtonElement>;
  onSelect: () => void;
}

function ModelRow({ model, onSelect, selected, selectedRef }: ModelRowProps) {
  const disabled = model.disabled === true;

  return (
    <button
      ref={selectedRef}
      aria-disabled={disabled || undefined}
      aria-selected={selected}
      className={styles.row}
      data-selected={selected ? "true" : undefined}
      disabled={disabled}
      role="option"
      type="button"
      onClick={disabled ? undefined : onSelect}
    >
      <span className={styles.rowContent}>
        <span className={styles.rowHeader}>
          <span className={styles.modelName}>{model.displayName}</span>
          {model.badges?.length ? (
            <span className={styles.badgeGroup}>
              {model.badges.map((badge) => (
                <Badge
                  key={badge}
                  className={styles.badge}
                  tone={badgeTone[badge]}
                >
                  {badgeLabel[badge]}
                </Badge>
              ))}
            </span>
          ) : null}
        </span>
        <span className={styles.metaRow}>
          {model.pricing ? (
            <span
              className={styles.metaItem}
              title="Price per 1M tokens, prompt/output"
            >
              {model.pricing.prompt} / {model.pricing.output}
            </span>
          ) : null}
          {model.contextWindow ? (
            <span className={styles.metaItem} title="Context window">
              {model.contextWindow}
            </span>
          ) : null}
          {model.capabilities ? (
            <span className={styles.capabilities}>{model.capabilities}</span>
          ) : null}
        </span>
      </span>
      {selected ? <Icon icon={Check} size="sm" tone="accent" /> : null}
    </button>
  );
}

export function ModelSelector({
  allowUnset = false,
  disabled = false,
  models,
  onSelect,
  triggerSize = "md",
  unsetLabel,
  value,
  variant = "popover",
  ...props
}: ModelSelectorProps) {
  const [open, setOpen] = React.useState(false);
  const currentLabel = getCurrentLabel(models, value, allowUnset, unsetLabel);
  const handleSelect = React.useCallback(
    (nextValue: string) => {
      if (nextValue === "" && allowUnset) {
        onSelect("");
        return;
      }

      onSelect(nextValue);
    },
    [allowUnset, onSelect],
  );

  if (variant === "inline") {
    return (
      <ModelSelectorList
        {...props}
        allowUnset={allowUnset}
        disabled={disabled}
        models={models}
        unsetLabel={unsetLabel}
        value={value}
        onSelect={handleSelect}
      />
    );
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        <Button
          aria-label="Select model"
          className={styles.trigger}
          disabled={disabled}
          title={currentLabel}
          rightIcon={open ? X : ChevronDown}
          variant="soft"
          size={triggerSize}
        >
          <span className={styles.triggerText}>{currentLabel}</span>
        </Button>
      </Popover.Trigger>
      <Popover.Content
        align="start"
        maxHeight="min(520px, calc(100dvh - var(--rf-space-6)))"
        maxWidth="min(420px, calc(100vw - var(--rf-space-4)))"
        scrollable={false}
      >
        <ModelSelectorList
          {...props}
          allowUnset={allowUnset}
          disabled={disabled}
          models={models}
          unsetLabel={unsetLabel}
          value={value}
          onRequestClose={() => setOpen(false)}
          onSelect={handleSelect}
        />
      </Popover.Content>
    </Popover>
  );
}
