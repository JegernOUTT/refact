import React, { useMemo } from "react";
import classNames from "classnames";
import { CheckCircle2, CircleX } from "lucide-react";
import { Markdown, ShikiCodeBlock } from "../Markdown";
import { Badge, Icon } from "../ui";
import { AnimatedCollapsible } from "./shared/AnimatedCollapsible";
import styles from "./FinalReportView.module.css";

type VerificationResult = {
  command: string;
  exit_code?: number | null;
  passed: boolean;
  output_tail: string;
};
type SuggestedCard = { title: string; priority: string; instructions: string };
type FinalReport = {
  summary: string;
  success: boolean;
  files_changed: string[];
  tests_added_or_updated: string[];
  verification: VerificationResult[];
  followup_cards: SuggestedCard[];
  risks: string[];
  assumptions: string[];
};
type FinalReportViewProps = { content: string; title?: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function optionalStringArray(value: unknown): string[] | null {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) return null;
  return value.every((item) => typeof item === "string") ? value : null;
}

function parseVerification(value: unknown): VerificationResult[] | null {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) return null;
  const out: VerificationResult[] = [];
  for (const item of value) {
    if (!isRecord(item)) return null;
    const exitCode = item.exit_code;
    if (
      typeof item.command !== "string" ||
      typeof item.passed !== "boolean" ||
      typeof item.output_tail !== "string"
    ) {
      return null;
    }
    if (
      exitCode !== undefined &&
      exitCode !== null &&
      typeof exitCode !== "number"
    ) {
      return null;
    }
    out.push({
      command: item.command,
      passed: item.passed,
      output_tail: item.output_tail,
      exit_code: exitCode,
    });
  }
  return out;
}

function parseFollowups(value: unknown): SuggestedCard[] | null {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) return null;
  const out: SuggestedCard[] = [];
  for (const item of value) {
    if (!isRecord(item)) return null;
    if (
      typeof item.title !== "string" ||
      typeof item.priority !== "string" ||
      typeof item.instructions !== "string"
    ) {
      return null;
    }
    out.push({
      title: item.title,
      priority: item.priority,
      instructions: item.instructions,
    });
  }
  return out;
}

function parseFinalReport(content: string): FinalReport | null {
  let raw: unknown;
  try {
    raw = JSON.parse(content);
  } catch {
    return null;
  }
  if (
    !isRecord(raw) ||
    typeof raw.summary !== "string" ||
    typeof raw.success !== "boolean"
  ) {
    return null;
  }
  const files = optionalStringArray(raw.files_changed);
  const tests = optionalStringArray(raw.tests_added_or_updated);
  const verification = parseVerification(raw.verification);
  const followups = parseFollowups(raw.followup_cards);
  const risks = optionalStringArray(raw.risks);
  const assumptions = optionalStringArray(raw.assumptions);
  if (
    !files ||
    !tests ||
    !verification ||
    !followups ||
    !risks ||
    !assumptions
  ) {
    return null;
  }
  return {
    summary: raw.summary,
    success: raw.success,
    files_changed: files,
    tests_added_or_updated: tests,
    verification,
    followup_cards: followups,
    risks,
    assumptions,
  };
}

function truncate(text: string): string {
  return text.length > 220 ? `${text.slice(0, 219)}…` : text;
}

const Section: React.FC<{ title: string; children: React.ReactNode }> = ({
  title,
  children,
}) => (
  <section className={styles.section}>
    <div className={styles.sectionTitle}>{title}</div>
    {children}
  </section>
);

const TextDetails: React.FC<{ title: string; items: string[] }> = ({
  title,
  items,
}) =>
  items.length > 0 ? (
    <AnimatedCollapsible
      className={styles.details}
      defaultOpen={false}
      header={
        <span className={styles.collapsibleTitle}>
          {title} ({items.length})
        </span>
      }
      variant="compact"
    >
      <ul className={styles.list}>
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    </AnimatedCollapsible>
  ) : (
    <Section title={title}>
      <span className={styles.none}>None</span>
    </Section>
  );

const VerificationDetails: React.FC<{ item: VerificationResult }> = ({
  item,
}) => (
  <AnimatedCollapsible
    className={styles.verificationItem}
    defaultOpen={false}
    header={
      <span className={styles.verificationHeader}>
        <Icon
          icon={item.passed ? CheckCircle2 : CircleX}
          size="sm"
          tone={item.passed ? "success" : "danger"}
        />
        <code>{item.command}</code>
        {item.exit_code !== undefined && item.exit_code !== null && (
          <span className={styles.exitCode}>({item.exit_code})</span>
        )}
      </span>
    }
    variant="compact"
  >
    {item.output_tail && (
      <ShikiCodeBlock showLineNumbers={false}>
        {item.output_tail}
      </ShikiCodeBlock>
    )}
  </AnimatedCollapsible>
);

export const FinalReportView: React.FC<FinalReportViewProps> = ({
  content,
  title = "Final Report",
}) => {
  const report = useMemo(() => parseFinalReport(content), [content]);
  if (!report) {
    return (
      <div className={styles.legacy}>
        <Markdown>{content}</Markdown>
      </div>
    );
  }

  return (
    <div className={styles.root} data-testid="final-report-view">
      <header className={classNames(styles.header, "rf-enter")}>
        <span className={styles.title}>{title}</span>
        <Badge tone={report.success ? "success" : "danger"}>
          <Icon
            icon={report.success ? CheckCircle2 : CircleX}
            size="sm"
            tone={report.success ? "success" : "danger"}
          />
          {report.success ? "Success" : "Failed"}
        </Badge>
      </header>
      <Section title="Summary">
        <div className={styles.markdown}>
          <Markdown>{report.summary}</Markdown>
        </div>
      </Section>
      <Section title="Files changed">
        {report.files_changed.length > 0 ? (
          <div className={styles.followupHeader}>
            {report.files_changed.map((file) => (
              <Badge key={file} tone="muted">
                {file}
              </Badge>
            ))}
          </div>
        ) : (
          <span className={styles.none}>None</span>
        )}
      </Section>
      <Section title="Tests added or updated">
        {report.tests_added_or_updated.length > 0 ? (
          <ul className={styles.list}>
            {report.tests_added_or_updated.map((test) => (
              <li key={test}>{test}</li>
            ))}
          </ul>
        ) : (
          <span className={styles.none}>None</span>
        )}
      </Section>
      <Section title="Verification">
        {report.verification.length > 0 ? (
          <div>
            {report.verification.map((item) => (
              <VerificationDetails
                key={`${item.command}:${item.exit_code ?? ""}`}
                item={item}
              />
            ))}
          </div>
        ) : (
          <span className={styles.none}>None</span>
        )}
      </Section>
      <Section title="Followup cards">
        {report.followup_cards.length > 0 ? (
          <div>
            {report.followup_cards.map((card) => (
              <article key={card.title} className={styles.followupCard}>
                <div className={styles.followupHeader}>
                  <span className={styles.followupTitle}>{card.title}</span>
                  <Badge tone="muted">{card.priority}</Badge>
                </div>
                <p className={styles.followupInstructions}>
                  {truncate(card.instructions)}
                </p>
              </article>
            ))}
          </div>
        ) : (
          <span className={styles.none}>None</span>
        )}
      </Section>
      <TextDetails title="Risks" items={report.risks} />
      <TextDetails title="Assumptions" items={report.assumptions} />
    </div>
  );
};

export default FinalReportView;
