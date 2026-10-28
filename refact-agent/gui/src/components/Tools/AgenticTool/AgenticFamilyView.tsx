import type { ReactNode } from "react";
import { TranscriptClamp } from "../../ChatContent/TranscriptItem";
import { agenticCardLabel } from "./agenticFamilies";
import type { AgenticToolViewModel } from "./agenticToolModel";
import { WorkToolView } from "./WorkToolView";
import { ApprovalToolView } from "./ApprovalToolView";
import { ResumeToolView } from "./ResumeToolView";
import { MessageToolView } from "./MessageToolView";
import { FamilyReportView } from "./FamilyReportView";

function familyBody(
  model: AgenticToolViewModel,
  toolName: string,
  content: string | null,
  extra: Record<string, unknown> | undefined,
  argumentsRaw: string | undefined,
): ReactNode {
  if (model.status !== "success") return null;
  if (toolName === "work_overview" || toolName === "work_inspect")
    return <WorkToolView content={content} />;
  if (toolName === "agent_approval")
    return <ApprovalToolView content={content} />;
  if (toolName === "agent_resume" || toolName === "agent_lifecycle")
    return (
      <ResumeToolView
        lifecycle={toolName === "agent_lifecycle"}
        content={content}
      />
    );
  if (model.family === "message" || model.family === "progress")
    return (
      <MessageToolView
        toolName={toolName}
        argumentsRaw={argumentsRaw}
        content={content}
        extra={extra}
      />
    );
  return (
    <FamilyReportView
      model={model}
      content={content}
      label={agenticCardLabel(toolName) ?? undefined}
    />
  );
}

export function AgenticFamilyView({
  model,
  toolName,
  content,
  extra,
  argumentsRaw,
}: {
  model: AgenticToolViewModel;
  toolName: string;
  content: string | null;
  extra: Record<string, unknown> | undefined;
  argumentsRaw: string | undefined;
}) {
  const body = familyBody(model, toolName, content, extra, argumentsRaw);
  if (body === null) return null;
  return <TranscriptClamp>{body}</TranscriptClamp>;
}
