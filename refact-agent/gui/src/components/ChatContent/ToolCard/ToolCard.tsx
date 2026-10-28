import React from "react";
import classNames from "classnames";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { TranscriptItem } from "../TranscriptItem/TranscriptItem";
import {
  fromToolTone,
  type TranscriptStatus,
} from "../TranscriptItem/statusVocabulary";
import { ToolCall } from "../../../services/refact/types";
import styles from "./ToolCard.module.css";

export type ToolStatus = "idle" | "running" | "success" | "error";

export interface ToolCardProps {
  icon: React.ReactNode;
  summary: React.ReactNode;
  verb?: React.ReactNode;
  meta?: React.ReactNode;
  density?: "default" | "compact";
  actions?: React.ReactNode;
  status: ToolStatus;
  transcriptStatus?: TranscriptStatus;
  statusLabel?: React.ReactNode;
  isOpen: boolean;
  onToggle: () => void;
  collapsible?: boolean;
  children?: React.ReactNode;
  className?: string;
  animate?: boolean;
  toolCall?: ToolCall;
}

const HIDDEN_OUTPUT_LABEL = { output: "" };

const ToolCardInner: React.FC<ToolCardProps> = ({
  icon,
  summary,
  verb,
  meta,
  density = "default",
  actions,
  status,
  transcriptStatus,
  statusLabel,
  isOpen,
  onToggle,
  collapsible = true,
  children,
  className,
  animate = true,
  toolCall,
}) => {
  const hasBody =
    children !== null && children !== undefined && children !== false;
  const handleOpenChange = React.useCallback(() => onToggle(), [onToggle]);

  const card = (
    <TranscriptItem
      animate={animate}
      className={classNames("rf-enter", styles.card, className)}
      data-density={density}
      data-status={status}
      expandable={collapsible}
      icon={icon}
      meta={meta}
      open={collapsible ? isOpen : true}
      output={hasBody ? <div className={styles.content}>{children}</div> : null}
      quickAction={actions}
      sectionLabels={HIDDEN_OUTPUT_LABEL}
      status={transcriptStatus ?? fromToolTone(status)}
      statusLabel={statusLabel}
      target={summary}
      verb={verb}
      onOpenChange={handleOpenChange}
    />
  );

  return toolCall ? (
    <ToolCallTooltip toolCall={toolCall}>{card}</ToolCallTooltip>
  ) : (
    card
  );
};

ToolCardInner.displayName = "ToolCard";

export const ToolCard = React.memo(ToolCardInner);

export default ToolCard;
