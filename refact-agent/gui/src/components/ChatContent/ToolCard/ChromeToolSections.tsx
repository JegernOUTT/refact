import React from "react";
import type {
  BrowserAssertionResult,
  BrowserExecutionStep,
} from "../../../services/refact/browser";
import { ShikiCodeBlock } from "../../Markdown";
import { DialogImage } from "../../DialogImage";
import { Badge } from "../../ui";
import {
  type FillDiagnosticChip,
  type StepCaptureEntry,
  formatFrameChip,
  prettifyActionName,
  renderAssertionValue,
} from "./ChromeToolModel";
import styles from "./ChromeTool.module.css";

export interface StepRow {
  stepIndex: number;
  ok: boolean;
  summary: string;
  locatorEcho: string | null;
  diagnostics: FillDiagnosticChip[];
}

export function StepList({ steps }: { steps: readonly string[] }) {
  return (
    <ol className={styles.stepList} data-testid="browser-request-steps">
      {steps.map((step, index) => (
        <li className={styles.stepItem} key={index}>
          {step}
        </li>
      ))}
    </ol>
  );
}

export function StepResults({ rows }: { rows: readonly StepRow[] }) {
  return (
    <ol className={styles.stepList} data-testid="browser-step-rows">
      {rows.map((row) => (
        <li
          className={styles.stepItem}
          data-status={row.ok ? "success" : "error"}
          key={row.stepIndex}
        >
          <span className={styles.stepSummary}>{row.summary}</span>
          {row.diagnostics.length > 0 ? (
            <span
              className={styles.chipRow}
              data-testid="browser-fill-diagnostics"
            >
              {row.diagnostics.map((chip) => (
                <Badge key={chip.label} size="xs" tone={chip.tone}>
                  {chip.label}
                </Badge>
              ))}
            </span>
          ) : null}
          {row.locatorEcho ? (
            <span
              className={styles.locatorEcho}
              data-testid="browser-locator-echo"
            >
              {row.locatorEcho}
            </span>
          ) : null}
        </li>
      ))}
    </ol>
  );
}

export function TextBlock({ label, text }: { label: string; text: string }) {
  return (
    <div className={styles.block}>
      <span className={styles.blockLabel}>{label}</span>
      <ShikiCodeBlock showLineNumbers={false}>{text}</ShikiCodeBlock>
    </div>
  );
}

export function WarningList({ warnings }: { warnings: readonly string[] }) {
  if (warnings.length === 0) return null;
  return (
    <div className={styles.warnings} data-testid="browser-warnings">
      {warnings.map((warning, index) => (
        <span key={index}>{warning}</span>
      ))}
    </div>
  );
}

export function AssertionList({
  entries,
}: {
  entries: readonly {
    step: BrowserExecutionStep;
    assertion: BrowserAssertionResult;
  }[];
}) {
  if (entries.length === 0) return null;
  return (
    <div className={styles.block}>
      <span className={styles.blockLabel}>Assertions</span>
      {entries.map(({ step, assertion }) => (
        <div
          className={styles.assertion}
          data-status={assertion.passed ? "success" : "error"}
          key={step.step_index}
        >
          <span className={styles.assertionHead}>
            <span className={styles.assertionStatus}>
              {assertion.passed ? "Passed" : "Failed"}
            </span>
            <span className={styles.mono}>
              {prettifyActionName(assertion.matcher)}
            </span>
            {assertion.soft ? <span className={styles.muted}>Soft</span> : null}
            <span className={styles.muted}>
              {assertion.attempts} attempt
              {assertion.attempts === 1 ? "" : "s"} · {assertion.elapsed_ms}
              ms
            </span>
          </span>
          <span className={styles.assertionValues}>
            <TextBlock
              label="Expected"
              text={renderAssertionValue(assertion.expected)}
            />
            <TextBlock
              label="Received"
              text={renderAssertionValue(assertion.received)}
            />
          </span>
          {assertion.diff ? (
            <TextBlock label="Diff" text={assertion.diff} />
          ) : null}
        </div>
      ))}
    </div>
  );
}

export function ScreenshotGrid({
  images,
  captures,
}: {
  images: readonly string[];
  captures: readonly StepCaptureEntry[];
}) {
  if (images.length === 0 && captures.length === 0) return null;
  return (
    <div className={styles.screenshotGrid} data-testid="browser-screenshots">
      {images.map((src, index) => (
        <DialogImage
          alt={`Screenshot ${index + 1}`}
          fallback=""
          key={`image-${index}`}
          size="8"
          src={src}
        />
      ))}
      {captures.map((capture) => (
        <div
          className={styles.capture}
          data-testid={`browser-capture-${capture.kind}`}
          key={`capture-${capture.stepIndex}`}
        >
          <span>
            {capture.label}
            {capture.detail ? (
              <span className={styles.muted}> · {capture.detail}</span>
            ) : null}
          </span>
          {capture.src ? (
            <DialogImage
              alt={capture.label}
              fallback=""
              size="8"
              src={capture.src}
            />
          ) : null}
          {capture.frames.length > 0 ? (
            <span className={styles.chipRow}>
              {capture.frames.map((frame) => (
                <Badge
                  key={frame.index}
                  size="xs"
                  title={frame.artifactPath ?? undefined}
                  tone="muted"
                >
                  {formatFrameChip(frame)}
                </Badge>
              ))}
            </span>
          ) : null}
          {capture.artifactPath ? (
            <span className={styles.mono} data-testid="capture-artifact">
              {capture.artifactPath}
            </span>
          ) : null}
          {capture.warnings.map((warning, index) => (
            <span className={styles.warningText} key={index}>
              {warning}
            </span>
          ))}
        </div>
      ))}
    </div>
  );
}

export function ErrorList({ lines }: { lines: readonly string[] }) {
  return (
    <ul className={styles.errorList}>
      {lines.map((line, index) => (
        <li key={index}>{line}</li>
      ))}
    </ul>
  );
}

export const Section: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => <div className={styles.sectionStack}>{children}</div>;
