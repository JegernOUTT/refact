import React, { useMemo, useState } from "react";
import {
  AppWindow,
  Camera,
  Database,
  Hammer,
  LayoutPanelLeft,
  Link,
  Radio,
  ScanSearch,
  Send,
  PanelsTopLeft,
} from "lucide-react";
import type { ToolCall, ToolResult } from "../../../services/refact/types";
import { Icon } from "../../ui/Icon";
import { DialogImage } from "../../DialogImage";
import { TranscriptFacts, TranscriptRaw } from "../TranscriptItem";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";
import { ToolCard } from "./ToolCard";
import {
  buildAppToolModel,
  isAppToolName,
  type AppToolName,
} from "./appToolModel";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import styles from "./AppTool.module.css";

const FAMILY_ICONS = {
  app: AppWindow,
  app_build: Hammer,
  app_state: Database,
  app_binding: Link,
  app_layout: LayoutPanelLeft,
  app_view: PanelsTopLeft,
  app_source: Radio,
  app_event: Send,
  app_inspect: ScanSearch,
  app_screenshot: Camera,
} satisfies Record<AppToolName, typeof AppWindow>;

interface AppToolProps {
  toolCall: ToolCall;
  // Only the response already permitted by the owning ToolsContent privacy branch.
  result?: ToolResult;
}
export const AppTool: React.FC<AppToolProps> = ({ toolCall, result }) => {
  const [open, setOpen] = useState(false);
  const unrecorded = useUnrecordedResultState(toolCall.id);
  const model = useMemo(
    () =>
      buildAppToolModel(
        toolCall.function.name,
        toolCall.function.arguments,
        result?.content,
        result !== undefined,
        result?.tool_failed === true,
      ),
    [toolCall.function.name, toolCall.function.arguments, result],
  );
  const status = result ? model.status : unrecorded.status;
  const outcome = result
    ? model.outcome
    : unrecorded.statusLabel ??
      (status === "running" ? "Pending" : "Not recorded");
  const family = normalizeToolName(toolCall.function.name);
  const familyIcon = isAppToolName(family) ? FAMILY_ICONS[family] : AppWindow;
  return (
    <div data-testid="app-tool" data-app-family={toolCall.function.name}>
      <ToolCard
        density="compact"
        icon={<Icon icon={familyIcon} />}
        summary={model.target ?? model.title}
        verb={model.target ? model.title : undefined}
        meta={status === "done" ? outcome : undefined}
        status={
          status === "failed"
            ? "error"
            : status === "running"
              ? "running"
              : "idle"
        }
        transcriptStatus={status}
        statusLabel={status === "done" ? undefined : outcome}
        isOpen={open}
        onToggle={() => setOpen((previous) => !previous)}
      >
        <div className={styles.body}>
          <TranscriptFacts facts={model.facts} />
          {model.sections.map((section, index) => (
            <section
              key={`${section.title}-${index}`}
              className={styles.section}
            >
              <h4 className={styles.heading}>{section.title}</h4>
              <TranscriptFacts facts={section.rows} />
            </section>
          ))}
          {model.images.length > 0 ? (
            <div className={styles.images} data-testid="app-tool-images">
              {model.images.map((src, index) => (
                <DialogImage key={index} fallback="" size="8" src={src} />
              ))}
            </div>
          ) : null}
          {result ? (
            <>
              <p className={styles.note}>
                Recorded response, not a live health check. Previews are
                bounded; endpoints show origins and common sensitive keys are
                redacted. This is not a complete secret detector.
              </p>
              <div className={styles.scrollX}>
                <TranscriptRaw
                  label="Raw JSON"
                  blocks={[
                    {
                      label: "Permitted response (defensive redaction)",
                      value: model.raw,
                    },
                  ]}
                />
              </div>
            </>
          ) : null}
        </div>
      </ToolCard>
    </div>
  );
};
