import React from "react";
import classNames from "classnames";
import type { LucideIcon } from "lucide-react";
import { Icon } from "../Icon";
import { TranscriptItem } from "../../ChatContent/TranscriptItem/TranscriptItem";
import {
  fromToolTone,
  type TranscriptStatus,
} from "../../ChatContent/TranscriptItem/statusVocabulary";

export type ToolCardStatus =
  | "idle"
  | "running"
  | "success"
  | "error"
  | "streaming";

export interface ToolCardProps
  extends Omit<React.ComponentPropsWithoutRef<"section">, "title"> {
  title: React.ReactNode;
  icon?: LucideIcon;
  status?: ToolCardStatus;
  density?: "default" | "compact";
  actions?: React.ReactNode;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  collapsible?: boolean;
  transcriptStatus?: TranscriptStatus;
  verb?: React.ReactNode;
  meta?: React.ReactNode;
  animate?: boolean;
  openKey?: string;
  children?: React.ReactNode;
}

const statusTone: Record<
  ToolCardStatus,
  "muted" | "accent" | "success" | "danger"
> = {
  idle: "muted",
  running: "accent",
  success: "success",
  error: "danger",
  streaming: "accent",
};

const HIDDEN_OUTPUT_LABEL = { output: "" };

export function ToolCard({
  title,
  icon,
  status = "idle",
  transcriptStatus,
  verb,
  meta,
  density = "default",
  actions,
  open,
  defaultOpen = true,
  onOpenChange,
  collapsible = true,
  animate = true,
  openKey,
  children,
  className,
  ...props
}: ToolCardProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen);
  const isControlled = open !== undefined;
  const isOpen = !collapsible || (isControlled ? open : uncontrolledOpen);
  const resolvedStatus = transcriptStatus ?? fromToolTone(status);
  const hasBody =
    children !== null && children !== undefined && children !== false;

  const handleOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      if (!isControlled) setUncontrolledOpen(nextOpen);
      onOpenChange?.(nextOpen);
    },
    [isControlled, onOpenChange],
  );

  return (
    <TranscriptItem
      animate={animate}
      className={classNames("rf-enter", className)}
      data-density={density}
      data-status={status}
      expandable={collapsible}
      icon={icon ? <Icon icon={icon} tone={statusTone[status]} /> : undefined}
      keepMounted={hasBody}
      meta={meta}
      open={hasBody ? isOpen : false}
      openKey={openKey}
      output={hasBody ? children : undefined}
      quickAction={actions}
      sectionLabels={HIDDEN_OUTPUT_LABEL}
      status={resolvedStatus}
      target={title}
      verb={verb}
      onOpenChange={handleOpenChange}
      {...props}
    />
  );
}
