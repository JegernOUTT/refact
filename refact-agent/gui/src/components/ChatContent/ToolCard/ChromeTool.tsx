import { Monitor, Image } from "lucide-react";
import React, { useMemo } from "react";
import { Box, Flex } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ToolCall } from "../../../services/refact/types";
import type {
  BrowserActionResponse,
  BrowserExecutionStep,
} from "../../../services/refact/browser";
import { ShikiCodeBlock } from "../../Markdown";
import { DialogImage } from "../../DialogImage";
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
    Array.isArray(value.steps)
  );
}

function summarizeStep(step: BrowserExecutionStep): string {
  if (step.ok) return step.summary;
  return step.error ? `${step.summary}: ${step.error}` : step.summary;
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

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
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

  const icon = images.length > 0 ? <Image /> : <Monitor />;

  const typedStepsBlock = useMemo(() => {
    if (!typedArgs) return null;
    return JSON.stringify(typedArgs, null, 2);
  }, [typedArgs]);

  const typedResultsBlock = useMemo(() => {
    if (!typedResult) return null;
    return typedResult.steps.map(summarizeStep).join("\n");
  }, [typedResult]);

  const typedDiagnosticsBlock = useMemo(() => {
    if (!typedResult) return null;
    return JSON.stringify(typedResult, null, 2);
  }, [typedResult]);

  return (
    <ToolCard
      icon={icon}
      summary={summary}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
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

      {images.length > 0 && (
        <Flex py="2" gap="2" wrap="wrap">
          {images.map((url, idx) => (
            <DialogImage key={idx} src={url} fallback="" size="8" />
          ))}
        </Flex>
      )}

      {typedResultsBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Results</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedResultsBlock}
            </ShikiCodeBlock>
          </Box>
        </Box>
      )}

      {typedDiagnosticsBlock && (
        <Box className={styles.section}>
          <Box className={styles.sectionLabel}>Execution Report</Box>
          <Box className={styles.logContent}>
            <ShikiCodeBlock showLineNumbers={false}>
              {typedDiagnosticsBlock}
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
