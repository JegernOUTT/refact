import { Monitor, Image, FileText } from "lucide-react";
import React, { useMemo, useState } from "react";
import { Box, Flex } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { selectToolResultByThreadAndId } from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import { ToolCall } from "../../../services/refact/types";
import type {
  ActionabilityDiagnostics,
  BrowserActionResponse,
  BrowserAssertionResult,
  BrowserAriaSnapshot,
  BrowserAriaSnapshotNode,
  BrowserCaptureKind,
  BrowserConsoleCounts,
  BrowserExecutionStep,
  BrowserFrameRecord,
  BrowserPageContext,
  BrowserPageSnapshot,
  BrowserSnapshotArtifact,
  BrowserSnapshotBox,
  BrowserSnapshotGeneration,
} from "../../../services/refact/browser";
import { ShikiCodeBlock } from "../../Markdown";
import { DialogImage } from "../../DialogImage";
import { Badge, Chip } from "../../ui";
import { AriaSnapshotView } from "./AriaSnapshotView";
import { ActionabilityLog } from "./ActionabilityLog";
import { ArtifactsPanel } from "./ArtifactsPanel";
import { NetworkPanel } from "./NetworkPanel";
import { PageHeader } from "./PageHeader";
import { PageSnapshot } from "./PageSnapshot";
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
import styles from "./ChromeTool.module.css";

interface ChromeArgs {
  commands?: string;
  request?: {
    steps: Record<string, unknown>[];
    [key: string]: unknown;
  };
}

interface CommandStats {
  url: string | null;
  screenshotCount: number;
  actionCounts: Partial<Record<string, number>>;
  totalActions: number;
}

const ACTION_LABELS: Partial<Record<string, string>> = {
  navigate_to: "navigate",
  click_at_element: "click",
  fill_field: "fill",
  type_text_at: "type",
  press_key: "key",
  screenshot: "screenshot",
  eval: "eval",
  scroll_to: "scroll",
  html: "inspect",
  styles: "styles",
  wait_for: "wait",
  wait_for_selector: "wait",
  wait_for_navigation: "wait",
  tab_log: "log",
  open_tab: "tab",
  close_tab: "tab",
  list_tabs: "tabs",
  reload: "reload",
};

function parseCommandStats(commands: string): CommandStats {
  const lines = commands.split("\n").filter((l) => {
    const t = l.trim();
    return t && !t.startsWith("//") && !t.startsWith("#");
  });

  let url: string | null = null;
  let screenshotCount = 0;
  const actionCounts: Partial<Record<string, number>> = {};

  for (const line of lines) {
    const parts = line.trim().split(/\s+/);
    const cmd = parts[0];
    if (!cmd) continue;

    const label = ACTION_LABELS[cmd] ?? cmd;
    actionCounts[label] = (actionCounts[label] ?? 0) + 1;

    if (cmd === "navigate_to" && parts.length >= 3 && !url) {
      url = parts.slice(2).join(" ");
    }
    if (cmd === "screenshot") {
      screenshotCount++;
    }
  }

  return {
    url,
    screenshotCount,
    actionCounts,
    totalActions: lines.length,
  };
}

function formatUrl(url: string): string {
  return url.replace(/^file:\/\//, "");
}

interface ChromeToolProps {
  toolCall: ToolCall;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function parseChromeArgs(value: string): ChromeArgs {
  try {
    const parsed = JSON.parse(value) as unknown;
    if (!isRecord(parsed)) return {};

    const commands =
      typeof parsed.commands === "string" ? parsed.commands : undefined;
    const request =
      isRecord(parsed.request) && Array.isArray(parsed.request.steps)
        ? {
            ...parsed.request,
            steps: parsed.request.steps.filter(isRecord),
          }
        : undefined;

    return { commands, request };
  } catch {
    return {};
  }
}

function isBrowserActionResponse(
  value: unknown,
): value is BrowserActionResponse {
  return (
    isRecord(value) &&
    typeof value.ok === "boolean" &&
    Array.isArray(value.steps) &&
    (typeof value.stabilized === "boolean" || value.stabilized === undefined)
  );
}

function optionalBoolean(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function optionalNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function parseSnapshotBox(value: unknown): BrowserSnapshotBox | null {
  if (!isRecord(value)) return null;
  const x = optionalNumber(value.x);
  const y = optionalNumber(value.y);
  const width = optionalNumber(value.width);
  const height = optionalNumber(value.height);
  if (
    x === undefined ||
    y === undefined ||
    width === undefined ||
    height === undefined
  ) {
    return null;
  }
  return { x, y, width, height };
}

function parseAriaSnapshotNode(value: unknown): BrowserAriaSnapshotNode | null {
  if (!isRecord(value) || typeof value.role !== "string") return null;
  return {
    role: value.role,
    name: typeof value.name === "string" ? value.name : null,
    ref: typeof value.ref === "string" ? value.ref : null,
    box: parseSnapshotBox(value.box),
  };
}

function parseSnapshotGeneration(
  value: unknown,
): BrowserSnapshotGeneration | null {
  if (!isRecord(value)) return null;
  const documentGeneration = optionalNumber(value.document_generation);
  const frameGeneration = optionalNumber(value.frame_generation);
  if (documentGeneration === undefined || frameGeneration === undefined) {
    return null;
  }
  const refs: BrowserSnapshotGeneration["refs"] = {};
  if (isRecord(value.refs)) {
    for (const [reference, info] of Object.entries(value.refs)) {
      if (!isRecord(info) || typeof info.role !== "string") continue;
      refs[reference] = {
        role: info.role,
        name: typeof info.name === "string" ? info.name : null,
      };
    }
  }
  return {
    document_generation: documentGeneration,
    frame_generation: frameGeneration,
    refs,
  };
}

function parseAriaSnapshot(value: unknown): BrowserAriaSnapshot | null {
  if (!isRecord(value) || typeof value.yaml !== "string") return null;
  const nodes = Array.isArray(value.nodes)
    ? value.nodes
        .map(parseAriaSnapshotNode)
        .filter((node): node is BrowserAriaSnapshotNode => node !== null)
    : [];
  return {
    yaml: value.yaml,
    nodes,
    generation: parseSnapshotGeneration(value.generation),
  };
}

function parseConsoleCounts(value: unknown): BrowserConsoleCounts {
  if (!isRecord(value)) return { errors: 0, warnings: 0 };
  return {
    errors: optionalNumber(value.errors) ?? 0,
    warnings: optionalNumber(value.warnings) ?? 0,
  };
}

function parseSnapshotArtifact(value: unknown): BrowserSnapshotArtifact | null {
  if (
    !isRecord(value) ||
    typeof value.kind !== "string" ||
    typeof value.mime !== "string" ||
    typeof value.path !== "string"
  ) {
    return null;
  }
  return {
    kind: value.kind,
    mime: value.mime,
    path: value.path,
    bytes: optionalNumber(value.bytes) ?? 0,
  };
}

function parsePageSnapshot(value: unknown): BrowserPageSnapshot | null {
  if (!isRecord(value) || typeof value.yaml !== "string") return null;
  return {
    yaml: value.yaml,
    lines: optionalNumber(value.lines) ?? 0,
    bytes: optionalNumber(value.bytes) ?? 0,
    truncated: value.truncated === true,
    artifact: parseSnapshotArtifact(value.artifact),
  };
}

function parsePageContext(value: unknown): BrowserPageContext | null {
  if (!isRecord(value)) return null;
  return {
    status: optionalNumber(value.status) ?? null,
    console: parseConsoleCounts(value.console),
    snapshot: parsePageSnapshot(value.snapshot),
  };
}

const CAPTURE_LABELS: Record<BrowserCaptureKind, string> = {
  filmstrip: "Filmstrip",
  element_gallery: "Element gallery",
  element_states: "Element states",
};

function captureKind(data: Record<string, unknown>): BrowserCaptureKind | null {
  if (isRecord(data.artifact) && data.artifact.kind === "filmstrip") {
    return "filmstrip";
  }
  if (Array.isArray(data.states) && Array.isArray(data.images)) {
    return "element_states";
  }
  if (typeof data.compose === "string" && Array.isArray(data.images)) {
    return "element_gallery";
  }
  return null;
}

function stringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === "string");
}

interface CaptureFrame extends BrowserFrameRecord {
  artifactPath: string | null;
}

function artifactPath(value: unknown): string | null {
  if (!isRecord(value) || !isRecord(value.artifact)) return null;
  return typeof value.artifact.path === "string" ? value.artifact.path : null;
}

function parseFrameRecords(value: unknown): CaptureFrame[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry) => {
    if (!isRecord(entry)) return [];
    const index = optionalNumber(entry.index);
    const offset = optionalNumber(entry.offset_ms);
    if (index === undefined || offset === undefined) return [];
    return [
      {
        index,
        offset_ms: offset,
        changed_percent: optionalNumber(entry.changed_percent) ?? null,
        artifactPath: artifactPath(entry),
      },
    ];
  });
}

function captureDetail(
  kind: BrowserCaptureKind,
  data: Record<string, unknown>,
): string | null {
  const parts: string[] = [];
  if (kind === "filmstrip") {
    const frames = optionalNumber(data.frame_count);
    const columns = optionalNumber(data.columns);
    const rows = optionalNumber(data.rows);
    const duration = optionalNumber(data.duration_ms);
    if (frames !== undefined) {
      parts.push(`${frames} frame${frames === 1 ? "" : "s"}`);
    }
    if (columns !== undefined && rows !== undefined) {
      parts.push(`${columns}×${rows} grid`);
    }
    if (duration !== undefined) parts.push(`${duration}ms`);
  } else if (kind === "element_gallery") {
    const count = optionalNumber(data.count);
    if (count !== undefined) {
      parts.push(`${count} element${count === 1 ? "" : "s"}`);
    }
    if (typeof data.compose === "string") parts.push(data.compose);
  } else {
    parts.push(...stringList(data.states));
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}

function isAttachableImageStep(
  report: BrowserActionResponse,
  data: Record<string, unknown>,
): boolean {
  const artifactKind = isRecord(data.artifact) ? data.artifact.kind : undefined;
  if (report.screenshot && artifactKind !== "filmstrip") return false;
  return (
    typeof data.mime === "string" &&
    data.mime.startsWith("image/") &&
    typeof data.data === "string"
  );
}

interface StepCaptureEntry {
  stepIndex: number;
  kind: BrowserCaptureKind;
  label: string;
  detail: string | null;
  frames: CaptureFrame[];
  warnings: string[];
  artifactPath: string | null;
  src?: string;
}

function formatFrameChip(frame: BrowserFrameRecord): string {
  const changed =
    frame.changed_percent === null || frame.changed_percent === undefined
      ? null
      : `${frame.changed_percent.toFixed(1)}% changed`;
  return changed
    ? `+${frame.offset_ms}ms · ${changed}`
    : `+${frame.offset_ms}ms`;
}

function parseActionabilityDiagnostics(
  value: unknown,
): ActionabilityDiagnostics | null {
  if (
    !isRecord(value) ||
    !Array.isArray(value.call_log) ||
    !value.call_log.every((entry) => typeof entry === "string") ||
    typeof value.timed_out !== "boolean"
  ) {
    return null;
  }

  return {
    call_log: value.call_log,
    timed_out: value.timed_out,
    elapsed_ms: optionalNumber(value.elapsed_ms),
    attempts: optionalNumber(value.attempts),
    attached: optionalBoolean(value.attached),
    visible: optionalBoolean(value.visible),
    stable: optionalBoolean(value.stable),
    enabled: optionalBoolean(value.enabled),
    editable: optionalBoolean(value.editable),
    receives_events: optionalBoolean(value.receives_events),
    intercepting_element:
      typeof value.intercepting_element === "string"
        ? value.intercepting_element
        : undefined,
  };
}

function parseAssertionResult(value: unknown): BrowserAssertionResult | null {
  if (
    !isRecord(value) ||
    typeof value.matcher !== "string" ||
    typeof value.passed !== "boolean" ||
    typeof value.soft !== "boolean" ||
    typeof value.attempts !== "number" ||
    typeof value.elapsed_ms !== "number"
  ) {
    return null;
  }

  return {
    matcher: value.matcher,
    passed: value.passed,
    soft: value.soft,
    expected: value.expected,
    received: value.received,
    diff: typeof value.diff === "string" ? value.diff : undefined,
    attempts: value.attempts,
    elapsed_ms: value.elapsed_ms,
  };
}

function renderAssertionValue(value: unknown): string {
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
}

function summarizeStep(step: BrowserExecutionStep): string {
  if (step.ok) return step.summary;
  return step.error ? `${step.summary}: ${step.error}` : step.summary;
}

interface FillDiagnosticChip {
  label: string;
  tone: "muted" | "success" | "warning";
}

function fillDiagnostics(step: BrowserExecutionStep): FillDiagnosticChip[] {
  const chips: FillDiagnosticChip[] = [];
  if (typeof step.field_kind === "string" && step.field_kind.length > 0) {
    chips.push({ label: step.field_kind, tone: "muted" });
  }
  if (typeof step.fill_strategy === "string" && step.fill_strategy.length > 0) {
    chips.push({ label: step.fill_strategy, tone: "muted" });
  }
  if (typeof step.verified === "boolean") {
    chips.push(
      step.verified
        ? { label: "verified", tone: "success" }
        : { label: "unverified", tone: "warning" },
    );
  }
  return chips;
}

function prettifyActionName(action: string): string {
  return action
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function describeTypedStep(step: Record<string, unknown>): string {
  const action = typeof step.action === "string" ? step.action : "step";

  if (action === "navigate" && typeof step.url === "string") {
    return `Navigate ${formatUrl(step.url)}`;
  }
  if (action === "fill" && isRecord(step.locator)) {
    const locator = step.locator;
    const by = typeof locator.by === "string" ? locator.by : "locator";
    const value =
      typeof locator.value === "string"
        ? locator.value
        : typeof locator.role === "string"
          ? locator.role
          : "element";
    return `Fill ${by}=${value}`;
  }
  if (action === "set_input_files" && isRecord(step.locator)) {
    const count = Array.isArray(step.paths) ? step.paths.length : 0;
    return `Upload ${count} file${count === 1 ? "" : "s"}`;
  }
  if (action === "expect_file_chooser") {
    const count = Array.isArray(step.paths) ? step.paths.length : 0;
    return `Arm file chooser for ${count} file${count === 1 ? "" : "s"}`;
  }
  if (action === "wait_for_download") return "Wait for download";
  if (action === "route_web_socket") return "Route WebSocket";
  if (action === "unroute_web_socket") return "Unroute WebSocket";
  if (action === "send_web_socket_message") return "Send WebSocket message";
  if (action === "wait_for_web_socket_frame") return "Wait for WebSocket frame";
  if (action === "start_har_recording") return "Start HAR recording";
  if (action === "stop_har_recording") return "Stop HAR recording";
  if (action === "route_from_har") return "Route from HAR";
  if (action === "start_coverage") return "Start JS/CSS coverage";
  if (action === "stop_coverage") return "Stop JS/CSS coverage";
  if (action === "add_virtual_authenticator")
    return "Add virtual authenticator";
  if (action === "remove_virtual_authenticator")
    return "Remove virtual authenticator";
  if (action === "list_credentials") return "List authenticator credentials";
  if (action === "add_credential") return "Add authenticator credential";
  if (action === "clear_credentials") return "Clear authenticator credentials";
  if (action === "set_user_verified") return "Set authenticator verification";
  if (action === "drag_and_drop") return "Drag element to target";
  if (action === "drop_files") {
    const count = Array.isArray(step.paths) ? step.paths.length : 0;
    return `Drop ${count} file${count === 1 ? "" : "s"}`;
  }
  if (
    (action === "mouse_move" || action === "mouse_click_xy") &&
    typeof step.x === "number" &&
    typeof step.y === "number"
  ) {
    return `${prettifyActionName(action)} (${step.x}, ${step.y})`;
  }
  if (action === "mouse_drag_xy") return "Mouse drag by coordinates";
  if (action === "mouse_wheel") return "Mouse wheel";
  if (
    (action === "click" || action === "scroll_to") &&
    isRecord(step.locator)
  ) {
    const locator = step.locator;
    const by = typeof locator.by === "string" ? locator.by : "locator";
    const value =
      typeof locator.value === "string"
        ? locator.value
        : typeof locator.role === "string"
          ? locator.role
          : "element";
    return `${prettifyActionName(action)} ${by}=${value}`;
  }
  return prettifyActionName(action);
}

export const ChromeTool: React.FC<ChromeToolProps> = ({ toolCall }) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);

  const threadId = useThreadId();
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );

  const args = useMemo(
    (): ChromeArgs => parseChromeArgs(toolCall.function.arguments),
    [toolCall.function.arguments],
  );

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult) return "running";
    if (
      typeof maybeResult === "object" &&
      "tool_failed" in maybeResult &&
      maybeResult.tool_failed
    ) {
      return "error";
    }
    return "success";
  }, [maybeResult]);

  const { textLog, images } = useMemo(() => {
    if (!maybeResult) return { textLog: null, images: [] as string[] };

    const content = maybeResult.content;

    if (typeof content === "string") {
      return { textLog: content || null, images: [] as string[] };
    }

    if (!Array.isArray(content)) {
      return { textLog: null, images: [] as string[] };
    }

    const textParts = content
      .filter(
        (item) =>
          isRecord(item) &&
          item.m_type === "text" &&
          typeof item.m_content === "string",
      )
      .map((item) => item.m_content)
      .join("\n")
      .trim();

    const imageParts = content
      .filter(
        (item) =>
          isRecord(item) &&
          typeof item.m_type === "string" &&
          item.m_type.startsWith("image/") &&
          typeof item.m_content === "string",
      )
      .map((item) => `data:${item.m_type};base64,${item.m_content}`);

    return { textLog: textParts || null, images: imageParts };
  }, [maybeResult]);

  const typedResult = useMemo<BrowserActionResponse | null>(() => {
    if (!textLog) return null;
    try {
      const parsed = JSON.parse(textLog) as unknown;
      return isBrowserActionResponse(parsed) ? parsed : null;
    } catch {
      return null;
    }
  }, [textLog]);

  const typedArgs = useMemo(() => {
    return args.request ?? null;
  }, [args.request]);

  const stats = useMemo(
    () => parseCommandStats(args.commands ?? ""),
    [args.commands],
  );

  const summary = useMemo(() => {
    if (typedArgs) {
      const stepDescriptions = typedArgs.steps
        .slice(0, 3)
        .filter(isRecord)
        .map(describeTypedStep);
      const moreCount = typedArgs.steps.length - stepDescriptions.length;
      return (
        <>
          Browser action
          {stepDescriptions.length > 0
            ? ` · ${stepDescriptions.join(", ")}`
            : ""}
          {moreCount > 0 ? ` · +${moreCount} more` : ""}
        </>
      );
    }

    const effectiveScreenshots = maybeResult
      ? images.length
      : stats.screenshotCount;
    const urlLabel = stats.url ? (
      <span className={styles.url}>{formatUrl(stats.url)}</span>
    ) : null;

    const parts: React.ReactNode[] = [];
    if (urlLabel) parts.push(urlLabel);

    const actionEntries: [string, number][] = [];
    for (const [key, count] of Object.entries(stats.actionCounts)) {
      if (key !== "screenshot" && count != null) {
        actionEntries.push([key, count]);
      }
    }
    if (actionEntries.length > 0) {
      const actionSummary = actionEntries
        .map(([key, count]) => (count > 1 ? `${count} ${key}` : key))
        .join(", ");
      parts.push(<span className={styles.meta}>{actionSummary}</span>);
    }

    if (effectiveScreenshots > 0) {
      parts.push(
        <span className={styles.meta}>
          {effectiveScreenshots} screenshot
          {effectiveScreenshots !== 1 ? "s" : ""}
        </span>,
      );
    }

    if (parts.length === 0) {
      return <>Browser commands</>;
    }

    return (
      <>
        Browser{" "}
        {parts.map((part, i) => (
          <React.Fragment key={i}>
            {i > 0 ? " · " : ""}
            {part}
          </React.Fragment>
        ))}
      </>
    );
  }, [typedArgs, stats, maybeResult, images]);

  const typedStepsBlock = useMemo(() => {
    if (!typedArgs) return null;
    return JSON.stringify(
      typedArgs,
      function (
        this: Record<string, unknown>,
        key: string,
        value: unknown,
      ): unknown {
        if (
          key === "password" ||
          key === "private_key" ||
          key === "credential_id" ||
          key === "user_handle" ||
          key === "large_blob" ||
          key === "user_name" ||
          key === "user_display_name"
        )
          return "[REDACTED]";
        if (key === "value" && typeof this.name === "string") {
          return "[REDACTED]";
        }
        return value;
      },
      2,
    );
  }, [typedArgs]);

  const typedResultsBlock = useMemo(() => {
    if (!typedResult) return null;
    return typedResult.steps.map(summarizeStep).join("\n");
  }, [typedResult]);

  const typedStepRows = useMemo(() => {
    if (!typedResult) return null;
    const rows = typedResult.steps.map((step) => ({
      stepIndex: step.step_index,
      summary: summarizeStep(step),
      locatorEcho:
        typeof step.locator_echo === "string" && step.locator_echo.length > 0
          ? step.locator_echo
          : null,
      diagnostics: fillDiagnostics(step),
    }));
    return rows.some(
      (row) => row.locatorEcho !== null || row.diagnostics.length > 0,
    )
      ? rows
      : null;
  }, [typedResult]);

  const reportWarnings = useMemo(
    () => (typedResult ? stringList(typedResult.warnings) : []),
    [typedResult],
  );

  const pageContext = useMemo(
    () => (typedResult ? parsePageContext(typedResult.page) : null),
    [typedResult],
  );

  const families = useMemo(
    () => collectBrowserFamilies(typedResult, typedArgs?.steps ?? null),
    [typedResult, typedArgs],
  );

  const { captures, usedImageIndexes } = useMemo(() => {
    const entries: StepCaptureEntry[] = [];
    const used = new Set<number>();
    if (!typedResult) return { captures: entries, usedImageIndexes: used };

    let cursor = 0;
    for (const step of typedResult.steps) {
      const data: unknown = step.data;
      if (!isRecord(data)) continue;
      const attachable = isAttachableImageStep(typedResult, data);
      const imageIndex = attachable ? cursor : null;
      if (attachable) cursor += 1;
      const kind = captureKind(data);
      if (!kind) continue;
      const src = imageIndex === null ? undefined : images[imageIndex];
      if (imageIndex !== null && src !== undefined) used.add(imageIndex);
      entries.push({
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
    return { captures: entries, usedImageIndexes: used };
  }, [typedResult, images]);

  const looseImages = useMemo(
    () => images.filter((_, index) => !usedImageIndexes.has(index)),
    [images, usedImageIndexes],
  );

  const consoleCollapsible = Boolean(
    pageContext &&
      (pageContext.console.errors > 0 || pageContext.console.warnings > 0),
  );
  const [consoleOpen, setConsoleOpen] = useState(false);
  const showConsoleSections = !consoleCollapsible || consoleOpen;

  const typedDiagnosticsBlock = useMemo(() => {
    if (!typedResult) return null;
    const lines = [
      typedResult.title ? `Title: ${typedResult.title}` : null,
      typedResult.url ? `URL: ${typedResult.url}` : null,
      `DOM stabilized: ${typedResult.stabilized === false ? "No" : "Yes"}`,
    ];
    return lines.filter((line): line is string => line !== null).join("\n");
  }, [typedResult]);

  const typedConsoleBlock = useMemo(() => {
    if (!typedResult?.console?.length) return null;
    return typedResult.console
      .map((entry) => `[${entry.level}] ${entry.text}`)
      .join("\n");
  }, [typedResult]);

  const typedPageErrorsBlock = useMemo(() => {
    if (!typedResult?.page_errors?.length) return null;
    return typedResult.page_errors.join("\n");
  }, [typedResult]);

  const typedLocatorHandlers = useMemo(() => {
    if (!typedResult?.locator_handlers?.length) return null;
    return typedResult.locator_handlers;
  }, [typedResult]);

  const typedAriaSnapshots = useMemo(() => {
    if (!typedResult) return [];
    return typedResult.steps.flatMap((step) => {
      const snapshot = parseAriaSnapshot(step.data);
      return snapshot ? [{ stepIndex: step.step_index, snapshot }] : [];
    });
  }, [typedResult]);

  const typedActionability = useMemo(() => {
    if (!typedResult) return [];
    return typedResult.steps.flatMap((step) => {
      const diagnostics = parseActionabilityDiagnostics(step.actionability);
      return diagnostics ? [{ step, diagnostics }] : [];
    });
  }, [typedResult]);

  const typedAssertions = useMemo(() => {
    if (!typedResult) return [];
    return typedResult.steps.flatMap((step) => {
      const assertion = parseAssertionResult(step.assertion);
      return assertion ? [{ step, assertion }] : [];
    });
  }, [typedResult]);

  const typedArtifacts = useMemo(() => {
    if (!typedResult) return [];
    return typedResult.steps.flatMap((step) => {
      const data: unknown = step.data;
      return isRecord(data) && isRecord(data.artifact) ? [data] : [];
    });
  }, [typedResult]);

  const typedDialogsBlock = useMemo(() => {
    if (!typedResult?.dialogs?.length) return null;
    return typedResult.dialogs
      .map((dialog) => {
        const handling = dialog.automatic
          ? `auto-${dialog.action}`
          : dialog.action;
        const defaultValue = dialog.default_value
          ? ` (default: ${dialog.default_value})`
          : "";
        return `[${dialog.type}] ${handling}: ${dialog.message}${defaultValue}`;
      })
      .join("\n");
  }, [typedResult]);

  const typedUploadsBlock = useMemo(() => {
    if (!typedResult?.uploads?.length) return null;
    return typedResult.uploads
      .map((upload) => {
        const paths = upload.paths.join(", ");
        const payload = upload.in_memory_payloads ? "in-memory" : "host paths";
        return `${upload.source}: ${paths} (${payload})`;
      })
      .join("\n");
  }, [typedResult]);

  const typedNewTabsBlock = useMemo(() => {
    if (!typedResult?.new_tabs?.length) return null;
    return typedResult.new_tabs
      .map((tab) => {
        const opener = tab.opener
          ? ` · opener ${tab.opener.tab_id}${
              tab.opener.frame_id ? ` frame ${tab.opener.frame_id}` : ""
            }`
          : "";
        const step =
          tab.opened_by_step === undefined || tab.opened_by_step === null
            ? ""
            : ` · step ${tab.opened_by_step + 1}`;
        return `${tab.active ? "Active" : "Opened"}: ${
          tab.title || tab.url
        }\n  ${tab.url}\n  ${tab.id}${opener}${step}`;
      })
      .join("\n");
  }, [typedResult]);

  const typedRoutesBlock = useMemo(() => {
    if (!typedResult?.active_routes?.length) return null;
    return typedResult.active_routes
      .map((route) => {
        const pattern =
          typeof route.pattern === "string"
            ? route.pattern
            : `/${route.pattern.source}/${route.pattern.flags ?? ""}`;
        return `${route.handler.type}: ${pattern}`;
      })
      .join("\n");
  }, [typedResult]);

  const typedInterceptionsBlock = useMemo(() => {
    if (!typedResult?.intercepted_requests?.length) return null;
    return typedResult.intercepted_requests
      .map((entry) => {
        const outcome = entry.status
          ? ` ${entry.status}`
          : entry.reason
            ? ` ${entry.reason}`
            : "";
        const redirect = entry.redirect_hop ? " · redirect hop" : "";
        return `${entry.action}${outcome}: ${entry.method} ${entry.url}${redirect}`;
      })
      .join("\n");
  }, [typedResult]);

  const typedContextBlock = useMemo(() => {
    const context = typedResult?.context;
    if (!context) return null;
    const identity = [
      context.viewport,
      context.locale,
      context.timezone,
      context.color_scheme,
    ].filter(Boolean);
    const permissions = context.permissions?.length
      ? `permissions: ${context.permissions.join(", ")}`
      : "permissions: none";
    return [
      identity.join(" · "),
      permissions,
      `cookies: ${context.cookie_count} · local storage: ${context.local_storage_count} · session storage: ${context.session_storage_count}`,
      context.offline ? "offline" : null,
      context.http_credentials ? "HTTP credentials: configured" : null,
    ]
      .filter(Boolean)
      .join("\n");
  }, [typedResult]);

  const reportScreenshot = typedResult?.screenshot
    ? `data:${typedResult.screenshot.mime};base64,${typedResult.screenshot.data}`
    : null;
  const hasImageArtifact = typedArtifacts.some(
    (data) => isRecord(data.artifact) && data.artifact.kind === "image",
  );
  const hasArtifacts =
    typedArtifacts.length > 0 || (typedResult?.downloads?.length ?? 0) > 0;

  const icon =
    images.length > 0 ||
    Boolean(typedResult?.screenshot) ||
    hasImageArtifact ? (
      <Image />
    ) : hasArtifacts ? (
      <FileText />
    ) : (
      <Monitor />
    );

  return (
    <ToolCard
      icon={icon}
      summary={summary}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      {pageContext && (
        <PageHeader
          console={pageContext.console}
          consoleOpen={showConsoleSections}
          onToggleConsole={() => setConsoleOpen((prev) => !prev)}
          status={pageContext.status}
          title={typedResult?.title}
          url={typedResult?.url}
        />
      )}

      {reportWarnings.length > 0 && (
        <Box className={styles.warningBanner} data-testid="browser-warnings">
          {reportWarnings.map((warning, index) => (
            <Box className={styles.warningLine} key={index}>
              {warning}
            </Box>
          ))}
        </Box>
      )}

      {typedStepsBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Request</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedStepsBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      {looseImages.length > 0 && (
        <Flex py="2" gap="2" wrap="wrap">
          {looseImages.map((url, idx) => (
            <DialogImage key={idx} src={url} fallback="" size="8" />
          ))}
        </Flex>
      )}

      {captures.length > 0 && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Captures</Box>
          <Box className={styles.captureList}>
            {captures.map((capture) => (
              <Box
                className={styles.capture}
                data-testid={`browser-capture-${capture.kind}`}
                key={capture.stepIndex}
              >
                <Flex align="baseline" gap="2" wrap="wrap">
                  <Box className={styles.captureLabel}>{capture.label}</Box>
                  {capture.detail && (
                    <Box className={styles.captureDetail}>{capture.detail}</Box>
                  )}
                </Flex>
                {capture.src && (
                  <DialogImage
                    alt={capture.label}
                    fallback=""
                    size="9"
                    src={capture.src}
                  />
                )}
                {capture.frames.length > 0 && (
                  <Flex gap="1" wrap="wrap">
                    {capture.frames.map((frame) => (
                      <Box
                        className={styles.frameChip}
                        key={frame.index}
                        title={frame.artifactPath ?? undefined}
                      >
                        {formatFrameChip(frame)}
                      </Box>
                    ))}
                  </Flex>
                )}
                {capture.artifactPath && (
                  <Chip data-testid="capture-artifact" radius="chip">
                    {capture.artifactPath}
                  </Chip>
                )}
                {capture.warnings.map((warning, index) => (
                  <Box className={styles.captureWarning} key={index}>
                    {warning}
                  </Box>
                ))}
              </Box>
            ))}
          </Box>
        </Box>
      )}

      {reportScreenshot && (
        <Flex py="2" gap="2" wrap="wrap">
          <DialogImage src={reportScreenshot} fallback="" size="8" />
        </Flex>
      )}

      <ArtifactsPanel
        artifacts={typedArtifacts}
        downloads={typedResult?.downloads}
      />

      {typedStepRows ? (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Results</Box>
          <Box className={styles.stepList} data-testid="browser-step-rows">
            {typedStepRows.map((row) => (
              <Box className={styles.stepRow} key={row.stepIndex}>
                <Box className={styles.stepSummary}>{row.summary}</Box>
                {row.diagnostics.length > 0 && (
                  <Flex
                    data-testid="browser-fill-diagnostics"
                    gap="1"
                    wrap="wrap"
                  >
                    {row.diagnostics.map((chip) => (
                      <Badge
                        key={chip.label}
                        size="xs"
                        tone={chip.tone}
                        variant="soft"
                      >
                        {chip.label}
                      </Badge>
                    ))}
                  </Flex>
                )}
                {row.locatorEcho && (
                  <Box
                    className={styles.locatorEcho}
                    data-testid="browser-locator-echo"
                  >
                    {row.locatorEcho}
                  </Box>
                )}
              </Box>
            ))}
          </Box>
        </Box>
      ) : (
        typedResultsBlock && (
          <Box className={styles.section}>
            <Box className={styles.sectionLabel}>Results</Box>
            <Box className={styles.logContent}>
              <ShikiCodeBlock showLineNumbers={false}>
                {typedResultsBlock}
              </ShikiCodeBlock>
            </Box>
          </Box>
        )
      )}

      <ReadoutPanel entries={families.readouts} />

      <HttpRequestPanel entries={families.httpRequests} />

      <CdpPanel entries={families.cdp} />

      {pageContext?.snapshot && (
        <PageSnapshot snapshot={pageContext.snapshot} />
      )}

      {typedDiagnosticsBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Page State</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedDiagnosticsBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      {showConsoleSections && typedConsoleBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Console</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedConsoleBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      {showConsoleSections && typedPageErrorsBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Page Errors</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedPageErrorsBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      <NetworkPanel entries={typedResult?.network} />

      <NetworkSummaryPanel rows={families.networkSummary} />

      <WebSocketPanel
        closes={families.webSocketCloses}
        events={families.webSocketEvents}
        routes={families.webSocketRoutes}
      />

      {typedContextBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Context</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedContextBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      <ClockTimeline entries={families.clock} />

      <DevicePanel catalogs={families.devices} emulation={families.emulation} />

      <ResetPanel entries={families.resets} />

      {typedRoutesBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Active Routes</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedRoutesBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      <RouteChainPanel chains={families.routeChains} />

      {typedInterceptionsBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Intercepted Requests</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedInterceptionsBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      {typedLocatorHandlers && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Locator Handlers</Box>
          <Box className={styles.handlerList}>
            {typedLocatorHandlers.map((handler, index) => (
              <Flex
                key={`${handler.name}-${index}`}
                align="baseline"
                gap="2"
                wrap="wrap"
                className={styles.handlerRow}
                data-status={handler.ok ? "success" : "error"}
              >
                <Box className={styles.handlerStatus}>
                  {handler.ok ? "Succeeded" : "Failed"}
                </Box>
                <Box className={styles.handlerName}>{handler.name}</Box>
                <Box className={styles.handlerDetail}>
                  Action: {handler.action}
                </Box>
                <Box className={styles.handlerDetail}>
                  Outcome: {handler.outcome}
                </Box>
              </Flex>
            ))}
          </Box>
        </Box>
      )}

      {typedAriaSnapshots.map(({ stepIndex, snapshot }) => (
        <Box className={styles.section} key={stepIndex}>
          <Box className={styles.sectionLabel}>ARIA Snapshot</Box>
          <AriaSnapshotView
            generation={snapshot.generation}
            nodes={snapshot.nodes}
            yaml={snapshot.yaml}
          />
        </Box>
      ))}

      {typedAssertions.length > 0 && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Assertions</Box>
          <Box className={styles.assertionList}>
            {typedAssertions.map(({ step, assertion }) => (
              <Box
                className={styles.assertion}
                data-status={assertion.passed ? "success" : "error"}
                key={step.step_index}
              >
                <Flex align="baseline" gap="2" wrap="wrap">
                  <Box className={styles.assertionStatus}>
                    {assertion.passed ? "Passed" : "Failed"}
                  </Box>
                  <Box className={styles.assertionName}>
                    {prettifyActionName(assertion.matcher)}
                  </Box>
                  {assertion.soft && (
                    <Box className={styles.assertionMeta}>Soft</Box>
                  )}
                  <Box className={styles.assertionMeta}>
                    {assertion.attempts} attempt
                    {assertion.attempts === 1 ? "" : "s"} ·{" "}
                    {assertion.elapsed_ms}ms
                  </Box>
                </Flex>
                <Box className={styles.assertionValues}>
                  <Box>
                    <Box className={styles.assertionValueLabel}>Expected</Box>
                    <ShikiCodeBlock showLineNumbers={false}>
                      {renderAssertionValue(assertion.expected)}
                    </ShikiCodeBlock>
                  </Box>
                  <Box>
                    <Box className={styles.assertionValueLabel}>Received</Box>
                    <ShikiCodeBlock showLineNumbers={false}>
                      {renderAssertionValue(assertion.received)}
                    </ShikiCodeBlock>
                  </Box>
                </Box>
                {assertion.diff && (
                  <Box>
                    <Box className={styles.assertionValueLabel}>Diff</Box>
                    <ShikiCodeBlock showLineNumbers={false}>
                      {assertion.diff}
                    </ShikiCodeBlock>
                  </Box>
                )}
              </Box>
            ))}
          </Box>
        </Box>
      )}

      {typedActionability.length > 0 && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Actionability</Box>
          <Box className={styles.actionabilityList}>
            {typedActionability.map(({ step, diagnostics }) => (
              <ActionabilityLog
                diagnostics={diagnostics}
                failed={!step.ok}
                key={step.step_index}
                retryCount={step.retries}
                stepIndex={step.step_index}
              />
            ))}
          </Box>
        </Box>
      )}

      {typedDialogsBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Dialogs</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedDialogsBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      {typedUploadsBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Uploads</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedUploadsBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      {typedNewTabsBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>New Tabs</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedNewTabsBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      {!typedResult && textLog && (
        <Box className={styles.logContent}>
          <ShikiCodeBlock showLineNumbers={false}>{textLog}</ShikiCodeBlock>
        </Box>
      )}
    </ToolCard>
  );
};

export default ChromeTool;
