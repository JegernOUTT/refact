import type { ReactNode } from "react";
import {
  Eye,
  EyeOff,
  MoreHorizontal,
  PanelRight,
  RefreshCw,
  Users,
} from "lucide-react";

import {
  Button,
  Icon,
  IconButton,
  Menu,
  SegmentedControl,
  Select,
} from "../../components/ui";
import type { Candidate } from "../../services/refact/review";
import type { DiffMode } from "../Workspace/GitPanel/diff/diffEditors";
import type { CompareMode } from "./model";
import { shortOid } from "./model";
import styles from "./ReviewToolbar.module.css";

type ReviewToolbarProps = {
  title: string;
  reference: ReactNode;
  candidate: Candidate | null;
  candidates: Candidate[];
  currentCandidateId: string | null;
  showUnchanged: boolean;
  sourcesOpen: boolean;
  workersOpen: boolean;
  sourcesId: string;
  workersId: string;
  activeWorkers: number;
  onCandidate: (candidateId: string) => void;
  onUnchanged: () => void;
  onSources: () => void;
  onWorkers: () => void;
  /** Present when the review belongs to a pull request. */
  onSyncPr?: () => void;
  handoff?: ReactNode;
  compact?: boolean;
};

export function ReviewToolbar({
  title,
  reference,
  candidate,
  candidates,
  currentCandidateId,
  showUnchanged,
  sourcesOpen,
  workersOpen,
  sourcesId,
  workersId,
  activeWorkers,
  onCandidate,
  onUnchanged,
  onSources,
  onWorkers,
  onSyncPr,
  handoff,
  compact = false,
}: ReviewToolbarProps) {
  const workersLabel =
    activeWorkers > 0 ? `Workers (${activeWorkers} active)` : "Workers";
  return (
    <div className={styles.toolbar} data-compact={compact}>
      <div className={styles.identity}>
        {!compact && (
          <h2 className={styles.title} title={title}>
            {title || "Review"}
          </h2>
        )}
        <div className={styles.receipt}>{reference}</div>
      </div>
      <div className={styles.actions} role="group" aria-label="Review controls">
        {candidate && candidates.length > 1 && (
          <Select value={candidate.id} onValueChange={onCandidate}>
            <Select.Trigger
              aria-label="Candidate revision"
              className={styles.candidate}
            />
            <Select.Content>
              {candidates.map((item, index) => (
                <Select.Item key={item.id} value={item.id}>
                  {`#${index + 1} ${shortOid(item.head_oid)}${
                    item.id === currentCandidateId ? " (current)" : ""
                  }`}
                </Select.Item>
              ))}
            </Select.Content>
          </Select>
        )}
        <span className={styles.trailing}>
          <Button
            type="button"
            size="sm"
            variant={sourcesOpen ? "soft" : "ghost"}
            aria-label="Sources & Evidence"
            aria-expanded={sourcesOpen}
            aria-controls={sourcesId}
            onClick={onSources}
          >
            <Icon icon={PanelRight} size="sm" /> Sources
          </Button>
          <Button
            type="button"
            size="sm"
            variant={workersOpen ? "soft" : "ghost"}
            aria-label={workersLabel}
            aria-expanded={workersOpen}
            aria-controls={workersId}
            onClick={onWorkers}
          >
            <Icon icon={Users} size="sm" /> Workers
            {activeWorkers > 0 ? ` · ${activeWorkers}` : ""}
          </Button>
          {handoff}
          <Menu>
            <Menu.Trigger asChild>
              <IconButton
                icon={MoreHorizontal}
                aria-label="More review options"
                size="sm"
                variant="ghost"
              />
            </Menu.Trigger>
            <Menu.Content align="end">
              <Menu.Item onSelect={onUnchanged}>
                <Icon icon={showUnchanged ? EyeOff : Eye} />
                {showUnchanged
                  ? "Hide unchanged files"
                  : "Show unchanged files"}
              </Menu.Item>
              {onSyncPr && (
                <Menu.Item onSelect={onSyncPr}>
                  <Icon icon={RefreshCw} />
                  Sync with the pull request
                </Menu.Item>
              )}
            </Menu.Content>
          </Menu>
        </span>
      </div>
    </div>
  );
}

/** What the diff compares and how it is laid out; sits with the file it applies to. */
export function ReviewDiffControls({
  candidate,
  currentCandidateId,
  compare,
  sinceLastAvailable,
  diffMode,
  onCompare,
  onDiffMode,
}: {
  candidate: Candidate;
  currentCandidateId: string | null;
  compare: CompareMode;
  sinceLastAvailable: boolean;
  diffMode: DiffMode;
  onCompare: (compare: CompareMode) => void;
  onDiffMode: (mode: DiffMode) => void;
}) {
  return (
    <>
      <span
        className={styles.comparison}
        title={`Base ${candidate.base_oid}\nHead ${candidate.head_oid}`}
        aria-label={`Base ${candidate.base_oid}, head ${candidate.head_oid}, ${
          candidate.id === currentCandidateId ? "current" : "historical"
        } candidate ${candidate.id}`}
      >
        {shortOid(candidate.base_oid)} <span aria-hidden="true">→</span>{" "}
        {shortOid(candidate.head_oid)}
      </span>
      {sinceLastAvailable && (
        <SegmentedControl
          size="sm"
          layout="content"
          name="Compare"
          aria-label="Comparison"
          value={compare}
          onValueChange={(value) =>
            onCompare(value === "since_last" ? "since_last" : "full")
          }
          options={[
            { value: "full", label: "Full" },
            { value: "since_last", label: "Since last" },
          ]}
        />
      )}
      <SegmentedControl
        size="sm"
        layout="content"
        name="Diff layout"
        aria-label="Diff layout"
        value={diffMode}
        onValueChange={(value) =>
          onDiffMode(value === "unified" ? "unified" : "side-by-side")
        }
        options={[
          { value: "side-by-side", label: "Split" },
          { value: "unified", label: "Unified" },
        ]}
      />
    </>
  );
}
