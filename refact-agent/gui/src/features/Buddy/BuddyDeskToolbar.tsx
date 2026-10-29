import type { Ref } from "react";
import { Info, Search, X } from "lucide-react";
import {
  FieldSelect,
  FieldText,
  Icon,
  IconButton,
  SegmentedControl,
  Tooltip,
} from "../../components/ui";
import {
  DESK_ATTENTION_OPTIONS,
  DESK_LIST_MODES,
  DESK_VIEWS,
  DESK_VIEW_LABELS,
  isDeskAttention,
  isDeskView,
  type DeskAttention,
  type DeskListMode,
} from "./BuddyDeskModel";
import type { BuddyReportsView } from "./sections/buddyReportsLink";
import { BuddyScrollRail } from "./ui";
import styles from "./BuddyDesk.module.css";

const GROUPING_HINT =
  "Reports are grouped when they share the same kind and subject. Only loaded reports are grouped.";

export function BuddyDeskViews({
  view,
  onChange,
  viewsRef,
}: {
  view: BuddyReportsView;
  onChange: (view: BuddyReportsView) => void;
  viewsRef: Ref<HTMLDivElement>;
}) {
  return (
    <BuddyScrollRail railRef={viewsRef} watch={view}>
      <SegmentedControl
        aria-label="Report views"
        layout="content"
        options={DESK_VIEWS.map((value) => ({
          value,
          label: DESK_VIEW_LABELS[value],
        }))}
        value={view}
        onValueChange={(value) => {
          if (isDeskView(value)) onChange(value);
        }}
      />
    </BuddyScrollRail>
  );
}

export function BuddyDeskFilters({
  search,
  showSearch,
  attention,
  onSearch,
  onAttention,
}: {
  search: string;
  showSearch: boolean;
  attention: DeskAttention;
  onSearch: (value: string) => void;
  onAttention: (value: DeskAttention) => void;
}) {
  return (
    <div className={styles.filters}>
      {showSearch && (
        <div className={styles.search}>
          <Icon
            icon={Search}
            size="sm"
            tone="muted"
            className={styles.searchIcon}
          />
          <FieldText
            type="search"
            aria-label="Search reports"
            placeholder="Search reports"
            className={styles.searchInput}
            value={search}
            maxLength={256}
            onChange={onSearch}
          />
          {search !== "" && (
            <IconButton
              icon={X}
              size="sm"
              variant="ghost"
              aria-label="Clear search"
              className={styles.searchClear}
              onClick={() => onSearch("")}
            />
          )}
        </div>
      )}
      <FieldSelect
        aria-label="Show reports"
        value={attention}
        options={DESK_ATTENTION_OPTIONS}
        onChange={(value) => {
          if (isDeskAttention(value)) onAttention(value);
        }}
      />
    </div>
  );
}

export function BuddyDeskListTools({
  listMode,
  showModes,
  count,
  grouped,
  onListMode,
}: {
  listMode: DeskListMode;
  showModes: boolean;
  count: string;
  grouped: boolean;
  onListMode: (mode: DeskListMode) => void;
}) {
  return (
    <div className={styles.listTools}>
      {showModes && (
        <BuddyScrollRail watch={listMode}>
          <SegmentedControl
            aria-label="List layout"
            layout="content"
            size="sm"
            options={DESK_LIST_MODES}
            value={listMode}
            onValueChange={(value) =>
              onListMode(value === "history" ? "history" : "latest")
            }
          />
        </BuddyScrollRail>
      )}
      <span className={styles.countGroup}>
        <span className={styles.count} data-testid="buddy-desk-count">
          {count}
        </span>
        {grouped && (
          <Tooltip content={GROUPING_HINT}>
            <IconButton
              icon={Info}
              size="sm"
              variant="ghost"
              aria-label={GROUPING_HINT}
            />
          </Tooltip>
        )}
      </span>
    </div>
  );
}
