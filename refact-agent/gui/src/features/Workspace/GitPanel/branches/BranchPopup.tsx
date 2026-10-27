import {
  ArrowDownToLine,
  ArrowUpFromLine,
  Check,
  ChevronDown,
  ChevronRight,
  CloudDownload,
  GitBranch,
  GitBranchPlus,
  GitCommitHorizontal,
  Tag,
  type LucideIcon,
} from "lucide-react";
import {
  useEffect,
  useId,
  useMemo,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import { Icon, Popover } from "../../../../components/ui";
import type { BranchRef } from "../log/branchesTreeModel";
import {
  buildPopupRows,
  isSelectable,
  type PopupAction,
  type PopupActionIcon,
  type PopupRow,
} from "./branchPopupModel";
import type { GitActionsController } from "./useGitActions";
import styles from "./Branches.module.css";

const POPUP_WIDTH = "calc(var(--rf-space-6) * 12)";
const POPUP_HEIGHT = "calc(var(--rf-space-6) * 16)";

const ACTION_ICONS: Record<PopupActionIcon, LucideIcon> = {
  update: ArrowDownToLine,
  commit: GitCommitHorizontal,
  push: ArrowUpFromLine,
  "new-branch": GitBranchPlus,
  "checkout-revision": Tag,
  fetch: CloudDownload,
};

function trackingHint(ref: BranchRef): string | null {
  if (ref.upstreamGone) return "gone";
  const parts: string[] = [];
  if (ref.ahead) parts.push(`↑${ref.ahead}`);
  if (ref.behind) parts.push(`↓${ref.behind}`);
  return parts.length > 0 ? parts.join(" ") : null;
}

function popupActions(
  controller: GitActionsController,
  onCommit: (() => void) | undefined,
): PopupAction[] {
  const blocked = controller.blockedReason ?? undefined;
  const updateBlocked = controller.updateBlockedReason ?? undefined;
  const actions: PopupAction[] = [
    {
      id: "update",
      icon: "update",
      label: "Update Project",
      onSelect: () => void controller.pull(),
      disabled: updateBlocked !== undefined,
      hint: updateBlocked,
    },
  ];
  if (onCommit) {
    actions.push({
      id: "commit",
      icon: "commit",
      label: "Commit…",
      onSelect: onCommit,
    });
  }
  actions.push(
    {
      id: "push",
      icon: "push",
      label: "Push…",
      onSelect: () => controller.openPush(),
      disabled: !controller.canPush,
      hint: controller.canPush ? undefined : "Check out a branch to push.",
    },
    {
      id: "new-branch",
      icon: "new-branch",
      label: "New Branch…",
      onSelect: () => controller.openNewBranch(),
    },
    {
      id: "checkout-revision",
      icon: "checkout-revision",
      label: "Checkout Tag or Revision…",
      onSelect: () => controller.openCheckoutRevision(),
      disabled: blocked !== undefined,
      hint: blocked,
    },
    {
      id: "fetch",
      icon: "fetch",
      label: "Fetch",
      onSelect: () => void controller.fetch(),
    },
  );
  return actions;
}

function rowLabel(row: PopupRow): string {
  if (row.kind === "header") return row.label;
  if (row.kind === "branch") return row.ref.name;
  return row.action.label;
}

type BranchPopupProps = {
  controller: GitActionsController;
  trigger: ReactNode;
  onCommit?: () => void;
};

export function BranchPopup({
  controller,
  trigger,
  onCommit,
}: BranchPopupProps) {
  const [open, setOpen] = useState(false);
  return (
    <Popover onOpenChange={setOpen} open={open} responsive={false}>
      <Popover.Trigger asChild>{trigger}</Popover.Trigger>
      {open ? (
        <Popover.Content
          align="start"
          className={styles.popup}
          maxHeight={POPUP_HEIGHT}
          maxWidth={POPUP_WIDTH}
          scrollable={false}
        >
          <BranchPopupBody
            controller={controller}
            onClose={() => setOpen(false)}
            onCommit={onCommit}
          />
        </Popover.Content>
      ) : null}
    </Popover>
  );
}

type BodyProps = {
  controller: GitActionsController;
  onClose: () => void;
  onCommit?: () => void;
};

function BranchPopupBody({ controller, onClose, onCommit }: BodyProps) {
  const listId = useId();
  const [query, setQuery] = useState("");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [activeId, setActiveId] = useState<string | null>(null);
  const { branchActions, branches, remotes } = controller;
  const actions = useMemo(
    () => popupActions(controller, onCommit),
    [controller, onCommit],
  );
  const rows = useMemo(
    () =>
      buildPopupRows({
        actions,
        branches,
        remotes,
        query,
        expandedId,
        branchActions,
      }),
    [actions, branchActions, branches, expandedId, query, remotes],
  );
  const selectable = useMemo(() => rows.filter(isSelectable), [rows]);
  const active =
    selectable.find((row) => row.id === activeId) ?? selectable.at(0) ?? null;
  const optionId = (id: string) => `${listId}-${encodeURIComponent(id)}`;

  useEffect(() => {
    if (!active) return;
    document
      .getElementById(optionId(active.id))
      ?.scrollIntoView({ block: "nearest" });
  });

  const run = (row: PopupRow) => {
    if (row.kind === "header" || !isSelectable(row)) return;
    onClose();
    if (row.kind === "branch") controller.checkoutBranchRef(row.ref);
    else row.action.onSelect();
  };

  const toggleExpanded = (id: string) => {
    setExpandedId((current) => (current === id ? null : id));
    setActiveId(id);
  };

  const move = (delta: number) => {
    if (selectable.length === 0) return;
    const index = active ? selectable.indexOf(active) : -1;
    const next = (index + delta + selectable.length) % selectable.length;
    setActiveId(selectable[next].id);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    switch (event.key) {
      case "ArrowDown":
        move(1);
        break;
      case "ArrowUp":
        move(-1);
        break;
      case "Home":
        if (selectable.length > 0) setActiveId(selectable[0].id);
        break;
      case "End":
        if (selectable.length > 0)
          setActiveId(selectable[selectable.length - 1].id);
        break;
      case "ArrowRight":
        if (active?.kind !== "branch" || expandedId === active.id) return;
        toggleExpanded(active.id);
        break;
      case "ArrowLeft":
        if (active?.kind === "branchAction") {
          setExpandedId(null);
          setActiveId(active.parentId);
        } else if (active?.kind === "branch" && expandedId === active.id) {
          setExpandedId(null);
        } else return;
        break;
      case "Enter":
        if (active) run(active);
        break;
      case "Escape":
        if (!expandedId) return;
        event.stopPropagation();
        setActiveId(
          active?.kind === "branchAction"
            ? active.parentId
            : active?.id ?? null,
        );
        setExpandedId(null);
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  return (
    <div className={styles.popupBody}>
      <input
        aria-activedescendant={active ? optionId(active.id) : undefined}
        aria-autocomplete="list"
        aria-controls={listId}
        aria-expanded
        aria-label="Search branches and actions"
        autoFocus
        className={styles.popupSearch}
        onChange={(event) => {
          setQuery(event.currentTarget.value);
          setActiveId(null);
          setExpandedId(null);
        }}
        onKeyDown={handleKeyDown}
        placeholder="Search branches and actions"
        role="combobox"
        type="text"
        value={query}
      />
      <div
        aria-label="Branches and actions"
        className={styles.popupList}
        id={listId}
        role="listbox"
      >
        {rows.length === 0 ? (
          <p className={styles.popupEmpty} role="status">
            Nothing matches &apos;{query}&apos;
          </p>
        ) : null}
        {rows.map((row) => (
          <PopupRowView
            active={row.id === active?.id}
            blockedReason={controller.blockedReason}
            expanded={row.id === expandedId}
            id={optionId(row.id)}
            key={row.id}
            onHover={() => {
              if (isSelectable(row)) setActiveId(row.id);
            }}
            onRun={() => run(row)}
            onToggle={() => toggleExpanded(row.id)}
            row={row}
          />
        ))}
      </div>
    </div>
  );
}

type RowViewProps = {
  row: PopupRow;
  id: string;
  active: boolean;
  expanded: boolean;
  blockedReason: string | null;
  onHover: () => void;
  onRun: () => void;
  onToggle: () => void;
};

function PopupRowView({
  row,
  id,
  active,
  expanded,
  blockedReason,
  onHover,
  onRun,
  onToggle,
}: RowViewProps) {
  if (row.kind === "header") {
    return (
      <div
        className={styles.popupHeader}
        data-depth={row.depth}
        role="presentation"
      >
        {row.label}
      </div>
    );
  }
  const disabled = !isSelectable(row);
  const common = {
    "aria-disabled": disabled || undefined,
    "aria-selected": active,
    className: styles.popupRow,
    "data-active": active ? "true" : undefined,
    "data-depth": row.kind === "action" ? undefined : String(row.depth),
    id,
    onClick: onRun,
    onMouseMove: onHover,
    role: "option",
  } as const;

  if (row.kind === "action" || row.kind === "branchAction") {
    const icon =
      row.kind === "action" ? ACTION_ICONS[row.action.icon] : undefined;
    return (
      <div
        {...common}
        aria-label={rowLabel(row)}
        data-danger={row.action.danger ? "true" : undefined}
      >
        <span className={styles.popupIcon}>
          {icon ? <Icon icon={icon} size="sm" tone="muted" /> : null}
        </span>
        <span className={styles.popupLabel}>{row.action.label}</span>
        {disabled && row.action.hint ? (
          <span className={styles.popupHint}>{row.action.hint}</span>
        ) : null}
      </div>
    );
  }

  const { ref } = row;
  const hint = trackingHint(ref);
  return (
    <div
      {...common}
      aria-expanded={expanded}
      aria-label={ref.current ? `${ref.name} (current)` : ref.name}
      data-current={ref.current ? "true" : undefined}
      title={blockedReason ?? ref.upstream ?? ref.name}
    >
      <span className={styles.popupIcon}>
        <Icon
          icon={ref.current ? Check : GitBranch}
          size="sm"
          tone={ref.current ? "accent" : "muted"}
        />
      </span>
      <span className={styles.popupLabel}>{ref.label}</span>
      {hint ? (
        <span
          className={styles.popupHint}
          data-gone={ref.upstreamGone ? "true" : undefined}
        >
          {hint}
        </span>
      ) : null}
      {/* An option must not own interactive descendants, so the expander is
          a pointer-only affordance; assistive tech uses aria-expanded on the
          option with ArrowRight/ArrowLeft from the search box. */}
      <span
        aria-hidden="true"
        className={styles.popupExpand}
        onClick={(event) => {
          event.stopPropagation();
          onToggle();
        }}
        title={`Actions for ${ref.name}`}
      >
        <Icon
          icon={expanded ? ChevronDown : ChevronRight}
          size="sm"
          tone="muted"
        />
      </span>
    </div>
  );
}
