import { FileText, Image, Monitor } from "lucide-react";
import React, { useMemo } from "react";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { selectToolResultByThreadAndId } from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import type { ToolCall } from "../../../services/refact/types";
import type { BrowserActionResponse } from "../../../services/refact/browser";
import { buildImageDataUrl } from "../../../utils/imageDataUrl";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptFact,
  type TranscriptRawBlock,
  type TranscriptStatus,
} from "../TranscriptItem";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { ArtifactsPanel } from "./ArtifactsPanel";
import { collectBrowserFamilies } from "./browserFamilies";
import {
  CdpPanel,
  ClockTimeline,
  DevicePanel,
  HttpRequestPanel,
  NetworkSummaryPanel,
  ReadoutPanel,
  ResetPanel,
  RouteChainPanel,
  WebSocketPanel,
} from "./BrowserFamilyPanels";
import {
  type StepCaptureEntry,
  CAPTURE_LABELS,
  ariaSnapshotYaml,
  artifactPath,
  captureDetail,
  captureKind,
  describeLegacyTarget,
  describeStepsTarget,
  describeTypedStep,
  fillDiagnostics,
  isAttachableImageStep,
  isBrowserActionResponse,
  isRecord,
  parseActionabilityDiagnostics,
  parseAssertionResult,
  parseChromeArgs,
  parseCommandStats,
  parseFrameRecords,
  parsePageContext,
  plural,
  redactedRequest,
  stringList,
  summarizeStep,
  urlHost,
  actionabilityText,
  failedStepLines,
} from "./ChromeToolModel";
import {
  AssertionList,
  ErrorList,
  ScreenshotGrid,
  Section,
  StepList,
  StepResults,
  TextBlock,
  WarningList,
} from "./ChromeToolSections";
import { reportTextBlocks } from "./chromeReportText";

interface ChromeToolProps {
  toolCall: ToolCall;
}

function splitContent(content: unknown): {
  textLog: string | null;
  images: (string | null)[];
} {
  if (typeof content === "string")
    return { textLog: content || null, images: [] };
  if (!Array.isArray(content)) return { textLog: null, images: [] };
  const text = content
    .filter(
      (item) =>
        isRecord(item) &&
        item.m_type === "text" &&
        typeof item.m_content === "string",
    )
    .map((item) => (isRecord(item) ? String(item.m_content) : ""))
    .join("\n")
    .trim();
  const images = content
    .filter(
      (item) =>
        isRecord(item) &&
        typeof item.m_type === "string" &&
        item.m_type.startsWith("image/"),
    )
    .map((item) =>
      isRecord(item) ? buildImageDataUrl(item.m_type, item.m_content) : null,
    );
  return { textLog: text || null, images };
}

function parseReport(textLog: string | null): BrowserActionResponse | null {
  if (!textLog) return null;
  try {
    const parsed: unknown = JSON.parse(textLog);
    return isBrowserActionResponse(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function collectCaptures(
  report: BrowserActionResponse | null,
  images: (string | null)[],
): { captures: StepCaptureEntry[]; used: Set<number> } {
  const captures: StepCaptureEntry[] = [];
  const used = new Set<number>();
  if (!report) return { captures, used };
  let cursor = 0;
  for (const step of report.steps) {
    const data: unknown = step.data;
    if (!isRecord(data)) continue;
    const attachable = isAttachableImageStep(report, data);
    const imageIndex = attachable ? cursor : null;
    if (attachable) cursor += 1;
    const kind = captureKind(data);
    if (!kind) continue;
    const src =
      imageIndex === null ? undefined : images[imageIndex] ?? undefined;
    if (imageIndex !== null && src !== undefined) used.add(imageIndex);
    captures.push({
      stepIndex: step.step_index,
      kind,
      label: CAPTURE_LABELS[kind],
      detail: captureDetail(kind, data),
      frames: kind === "filmstrip" ? parseFrameRecords(data.frames) : [],
      warnings: stringList(data.warnings),
      artifactPath: artifactPath(data),
      src,
    });
  }
  return { captures, used };
}

function isErrorLevel(level: string): boolean {
  return level.toLowerCase().startsWith("error");
}

export const ChromeTool: React.FC<ChromeToolProps> = ({ toolCall }) => {
  const threadId = useThreadId();
  const unrecorded = useUnrecordedResultState(toolCall.id);
  const result = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const args = useMemo(
    () => parseChromeArgs(toolCall.function.arguments),
    [toolCall.function.arguments],
  );
  const { textLog, images } = useMemo(
    () => splitContent(result?.content),
    [result],
  );
  const report = useMemo(() => parseReport(textLog), [textLog]);
  const requestSteps = useMemo(() => args.request?.steps ?? [], [args]);
  const stats = useMemo(
    () => parseCommandStats(args.commands ?? ""),
    [args.commands],
  );
  const pageContext = report ? parsePageContext(report.page) : null;
  const families = useMemo(
    () => collectBrowserFamilies(report, args.request?.steps ?? null),
    [report, args.request],
  );
  const { captures, used } = useMemo(
    () => collectCaptures(report, images),
    [report, images],
  );

  const reportScreenshot = report?.screenshot
    ? buildImageDataUrl(report.screenshot.mime, report.screenshot.data)
    : null;
  const screenshots = [
    ...images.filter(
      (url, index): url is string => url !== null && !used.has(index),
    ),
    ...(reportScreenshot ? [reportScreenshot] : []),
  ];
  const artifactData = report
    ? report.steps.flatMap((step) => {
        const data: unknown = step.data;
        return isRecord(data) && isRecord(data.artifact) ? [data] : [];
      })
    : [];

  const steps = report?.steps ?? [];
  const failedSteps = report ? failedStepLines(report) : [];
  const consoleEntries = report?.console ?? [];
  const consoleErrors = consoleEntries.filter((entry) =>
    isErrorLevel(entry.level),
  );
  const consoleErrorCount = Math.max(
    pageContext?.console.errors ?? 0,
    consoleErrors.length,
  );
  const consoleWarnings = pageContext?.console.warnings ?? 0;

  const status: TranscriptStatus = !result
    ? unrecorded.status
    : failedSteps.length > 0 || (result.tool_failed && !report)
      ? "failed"
      : "done";
  const tone =
    status === "running"
      ? "running"
      : status === "failed"
        ? "error"
        : !result
          ? "idle"
          : "success";

  const target = args.request
    ? describeStepsTarget(requestSteps)
    : describeLegacyTarget(stats);
  const meta = [
    urlHost(report?.url ?? stats.url),
    report?.title ? report.title : null,
    consoleErrorCount > 0 ? plural(consoleErrorCount, "console error") : null,
  ]
    .filter(Boolean)
    .join(" · ");

  const summaryFacts: (TranscriptFact | null)[] = [
    report?.title ? { label: "Page", value: report.title } : null,
    report?.url ? { label: "URL", value: report.url, title: report.url } : null,
    pageContext?.status
      ? { label: "HTTP", value: String(pageContext.status) }
      : null,
    report?.context?.viewport
      ? { label: "Viewport", value: report.context.viewport }
      : null,
    steps.length > 0
      ? {
          label: "Steps",
          value: `${steps.length - failedSteps.length}/${steps.length} ok`,
        }
      : null,
    pageContext !== null || consoleEntries.length > 0
      ? {
          label: "Console",
          value: `${plural(consoleErrorCount, "error")} · ${plural(
            consoleWarnings,
            "warning",
          )}`,
        }
      : null,
  ];
  const summary = summaryFacts.filter(
    (fact): fact is TranscriptFact => fact !== null,
  );

  const inputSteps = args.request
    ? requestSteps.map(describeTypedStep)
    : (args.commands ?? "")
        .split("\n")
        .map((line) => line.trim())
        .filter(
          (line) => line && !line.startsWith("//") && !line.startsWith("#"),
        );

  const stepRows = steps.map((step) => ({
    stepIndex: step.step_index,
    ok: step.ok,
    summary: summarizeStep(step),
    locatorEcho:
      typeof step.locator_echo === "string" && step.locator_echo.length > 0
        ? step.locator_echo
        : null,
    diagnostics: fillDiagnostics(step),
  }));
  const assertions = steps.flatMap((step) => {
    const assertion = parseAssertionResult(step.assertion);
    return assertion ? [{ step, assertion }] : [];
  });
  const actionability = steps.flatMap((step) => {
    const diagnostics = parseActionabilityDiagnostics(step.actionability);
    return diagnostics && diagnostics.call_log.length > 0
      ? [
          {
            label: `Step ${step.step_index + 1} actionability`,
            text: actionabilityText(step, diagnostics),
          },
        ]
      : [];
  });
  const textBlocks = report ? reportTextBlocks(report) : [];

  const output = report ? (
    <TranscriptClamp>
      <Section>
        <WarningList warnings={stringList(report.warnings)} />
        {stepRows.length > 0 ? <StepResults rows={stepRows} /> : null}
        <AssertionList entries={assertions} />
        {actionability.map((entry) => (
          <TextBlock key={entry.label} label={entry.label} text={entry.text} />
        ))}
        <ReadoutPanel entries={families.readouts} />
        <HttpRequestPanel entries={families.httpRequests} />
        <CdpPanel entries={families.cdp} />
        <NetworkSummaryPanel rows={families.networkSummary} />
        <WebSocketPanel
          closes={families.webSocketCloses}
          events={families.webSocketEvents}
          routes={families.webSocketRoutes}
        />
        <ClockTimeline entries={families.clock} />
        <DevicePanel
          catalogs={families.devices}
          emulation={families.emulation}
        />
        <ResetPanel entries={families.resets} />
        <RouteChainPanel chains={families.routeChains} />
        {textBlocks.map((block) => (
          <TextBlock key={block.label} label={block.label} text={block.text} />
        ))}
      </Section>
    </TranscriptClamp>
  ) : textLog ? (
    <TranscriptClamp>
      <TextBlock label="Log" text={textLog} />
    </TranscriptClamp>
  ) : null;

  const errorLines = [
    ...failedSteps,
    ...consoleErrors.map((entry) => `Console: ${entry.text}`),
    ...(report?.page_errors ?? []).map((line) => `Page error: ${line}`),
  ];
  const hasArtifacts =
    artifactData.length > 0 || (report?.downloads?.length ?? 0) > 0;
  const artifacts =
    screenshots.length > 0 || captures.length > 0 || hasArtifacts ? (
      <Section>
        <ScreenshotGrid captures={captures} images={screenshots} />
        <ArtifactsPanel
          artifacts={artifactData}
          downloads={report?.downloads}
        />
      </Section>
    ) : null;

  const snapshots = [
    pageContext?.snapshot
      ? {
          label: pageContext.snapshot.truncated
            ? `Accessibility snapshot (truncated${
                pageContext.snapshot.artifact
                  ? `, full: ${pageContext.snapshot.artifact.path}`
                  : ""
              })`
            : "Accessibility snapshot",
          value: pageContext.snapshot.yaml,
        }
      : null,
    ...steps.flatMap((step) => {
      const yaml = ariaSnapshotYaml(step.data);
      return yaml
        ? [{ label: `Step ${step.step_index + 1} ARIA snapshot`, value: yaml }]
        : [];
    }),
  ];
  const rawBlocks: (TranscriptRawBlock | null)[] = [
    args.request
      ? { label: "Request", value: redactedRequest(args.request) }
      : args.commands
        ? { label: "Commands", value: args.commands }
        : null,
    consoleEntries.length > 0
      ? {
          label: "Console log",
          value: consoleEntries
            .map((entry) => `[${entry.level}] ${entry.text}`)
            .join("\n"),
        }
      : null,
    report?.network?.length
      ? { label: "Network log", value: report.network }
      : null,
    ...snapshots,
    textLog ? { label: "Response", value: textLog } : null,
  ];
  const raw = rawBlocks.filter(
    (block): block is TranscriptRawBlock => block !== null,
  );

  const icon =
    screenshots.length > 0 || captures.length > 0 ? (
      <Image />
    ) : hasArtifacts ? (
      <FileText />
    ) : (
      <Monitor />
    );

  return (
    <ToolCallTooltip toolCall={toolCall}>
      <TranscriptItem
        artifacts={artifacts}
        className="rf-enter"
        data-status={tone}
        data-testid="chrome-tool"
        error={errorLines.length > 0 ? <ErrorList lines={errorLines} /> : null}
        icon={icon}
        input={inputSteps.length > 0 ? <StepList steps={inputSteps} /> : null}
        meta={meta || undefined}
        openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
        output={output}
        raw={raw}
        status={status}
        statusLabel={!result ? unrecorded.statusLabel : undefined}
        summary={summary}
        target={target}
        verb="Browser"
      />
    </ToolCallTooltip>
  );
};

export default ChromeTool;
