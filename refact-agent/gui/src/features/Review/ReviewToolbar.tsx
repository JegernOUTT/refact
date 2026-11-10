import type { ReactNode } from "react";
import {
  BookOpen,
  Columns2,
  Eye,
  EyeOff,
  FileDiff,
  History,
  List,
  MoreHorizontal,
  PanelRight,
  Pause,
  Play,
  Users,
} from "lucide-react";

import {
  Icon,
  IconButton,
  Menu,
  SegmentedControl,
  Select,
  Tooltip,
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
  compare: CompareMode;
  sinceLastAvailable: boolean;
  diffMode: DiffMode;
  followTour: boolean;
  hasTour: boolean;
  showUnchanged: boolean;
  sourcesOpen: boolean;
  workersOpen: boolean;
  sourcesId: string;
  workersId: string;
  activeWorkers: number;
  onCandidate: (candidateId: string) => void;
  onCompare: (compare: CompareMode) => void;
  onDiffMode: (mode: DiffMode) => void;
  onFollow: () => void;
  onUnchanged: () => void;
  onSources: () => void;
  onWorkers: () => void;
  handoff?: ReactNode;
  compact?: boolean;
};

function Tip({ label, children }: { label: string; children: ReactNode }) {
  return (
    <Tooltip>
      <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
      <Tooltip.Content>{label}</Tooltip.Content>
    </Tooltip>
  );
}

export function ReviewToolbar({
  title,
  reference,
  candidate,
  candidates,
  currentCandidateId,
  compare,
  sinceLastAvailable,
  diffMode,
  followTour,
  hasTour,
  showUnchanged,
  sourcesOpen,
  workersOpen,
  sourcesId,
  workersId,
  activeWorkers,
  onCandidate,
  onCompare,
  onDiffMode,
  onFollow,
  onUnchanged,
  onSources,
  onWorkers,
  handoff,
  compact = false,
}: ReviewToolbarProps) {
  return (
    <div className={styles.toolbar} data-compact={compact}>
      <div className={styles.identity}>
        {!compact && (
          <h2 className={styles.title} title={title}>
            {title || "Review"}
          </h2>
        )}
        <div className={styles.receipt}>
          {reference}
          {candidate && (
            <span
              title={`Base ${candidate.base_oid}\nHead ${candidate.head_oid}`}
              aria-label={`Base ${candidate.base_oid}, head ${
                candidate.head_oid
              }, ${
                candidate.id === currentCandidateId ? "current" : "historical"
              } candidate ${candidate.id}`}
            >
              {shortOid(candidate.base_oid)} <span aria-hidden="true">→</span>{" "}
              {shortOid(candidate.head_oid)}
            </span>
          )}
        </div>
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
        <Tip label="Comparison">
          <span className={styles.controlGroup}>
            <SegmentedControl
              size="sm"
              layout="content"
              name="Compare"
              value={compare}
              onValueChange={(value) =>
                onCompare(value === "since_last" ? "since_last" : "full")
              }
              options={[
                {
                  value: "full",
                  label: <Icon icon={FileDiff} />,
                  iconOnly: true,
                  ariaLabel: "Full",
                },
                {
                  value: "since_last",
                  label: <Icon icon={History} />,
                  iconOnly: true,
                  ariaLabel: "Since last",
                  disabled: !sinceLastAvailable,
                },
              ]}
            />
          </span>
        </Tip>
        <Tip label="Diff layout">
          <span className={styles.controlGroup}>
            <SegmentedControl
              size="sm"
              layout="content"
              name="Diff layout"
              value={diffMode}
              onValueChange={(value) =>
                onDiffMode(value === "unified" ? "unified" : "side-by-side")
              }
              options={[
                {
                  value: "side-by-side",
                  label: <Icon icon={Columns2} />,
                  iconOnly: true,
                  ariaLabel: "Split",
                },
                {
                  value: "unified",
                  label: <Icon icon={List} />,
                  iconOnly: true,
                  ariaLabel: "Unified",
                },
              ]}
            />
          </span>
        </Tip>
        {hasTour && (
          <Tip label={followTour ? "Stop following" : "Follow tour"}>
            <IconButton
              icon={followTour ? Pause : Play}
              aria-label={followTour ? "Stop following" : "Follow tour"}
              aria-pressed={followTour}
              size="sm"
              variant={followTour ? "soft" : "ghost"}
              onClick={onFollow}
            />
          </Tip>
        )}
        {handoff}
        <Tip label="Sources & Evidence">
          <IconButton
            icon={PanelRight}
            aria-label="Sources & Evidence"
            aria-expanded={sourcesOpen}
            aria-controls={sourcesId}
            size="sm"
            variant={sourcesOpen ? "soft" : "ghost"}
            onClick={onSources}
          />
        </Tip>
        <Tip
          label={
            activeWorkers > 0 ? `Workers (${activeWorkers} active)` : "Workers"
          }
        >
          <IconButton
            icon={Users}
            aria-label={
              activeWorkers > 0
                ? `Workers (${activeWorkers} active)`
                : "Workers"
            }
            aria-expanded={workersOpen}
            aria-controls={workersId}
            size="sm"
            variant={workersOpen ? "soft" : "ghost"}
            onClick={onWorkers}
          />
        </Tip>
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
            <Menu.Label>View</Menu.Label>
            <Menu.Item onSelect={onUnchanged}>
              <Icon icon={showUnchanged ? EyeOff : Eye} />
              {showUnchanged ? "Hide unchanged files" : "Show unchanged files"}
            </Menu.Item>
            <Menu.Separator />
            <Menu.Item disabled>
              <Icon icon={BookOpen} />
              Review stays local
            </Menu.Item>
          </Menu.Content>
        </Menu>
      </div>
    </div>
  );
}
