import React, { useMemo } from "react";
import {
  PlayIcon,
  StopIcon,
  ReloadIcon,
  InfoCircledIcon,
  FileTextIcon,
} from "@radix-ui/react-icons";
import { ToolCall } from "../../../services/refact/types";
import { StreamingToolCard } from "./StreamingToolCard";

interface ShellServiceArgs {
  service_name?: string;
  action?: string;
  command?: string;
  workdir?: string;
}

const ACTION_ICONS: Record<string, React.ReactNode> = {
  start: <PlayIcon />,
  stop: <StopIcon />,
  restart: <ReloadIcon />,
  status: <InfoCircledIcon />,
  logs: <FileTextIcon />,
};

interface ShellServiceToolProps {
  toolCall: ToolCall;
}

export const ShellServiceTool: React.FC<ShellServiceToolProps> = ({
  toolCall,
}) => {
  const args = useMemo<ShellServiceArgs>(() => {
    try {
      return JSON.parse(toolCall.function.arguments) as ShellServiceArgs;
    } catch {
      return {};
    }
  }, [toolCall.function.arguments]);

  const action = args.action ?? "manage";
  const serviceName = args.service_name ?? "service";
  const icon = ACTION_ICONS[action] ?? <PlayIcon />;

  const actionLabel = action.charAt(0).toUpperCase() + action.slice(1);
  const summary = `${actionLabel} ${serviceName}`;

  const meta = args.command
    ? args.command
    : args.workdir
      ? `in ${args.workdir}`
      : null;

  return (
    <StreamingToolCard
      toolCall={toolCall}
      icon={icon}
      summary={summary}
      meta={meta}
    />
  );
};

export default ShellServiceTool;
