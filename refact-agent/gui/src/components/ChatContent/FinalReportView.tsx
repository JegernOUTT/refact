import React, { useMemo } from "react";
import classNames from "classnames";
import { CheckCircle2, CircleX } from "lucide-react";
import { Markdown, ShikiCodeBlock } from "../Markdown";
import { Badge, Icon, ToolCard } from "../ui";
import { AnimatedCollapsible } from "./shared/AnimatedCollapsible";
import styles from "./FinalReportView.module.css";

import { CompactToolDetails } from "../Tools/AgenticTool/CompactToolDetails";
import {
  finalReportPresentation,
  parseFinalReport,
  verificationPassed,
  verificationSummary,
  type VerificationResult,
} from "./finalReportJson";

type FinalReportViewProps = {
  content: string;
  title?: string;
  compact?: boolean;
};

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
          icon={verificationPassed(item) ? CheckCircle2 : CircleX}
          size="sm"
          tone={verificationPassed(item) ? "success" : "muted"}
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
  compact = false,
}) => {
  const report = useMemo(() => parseFinalReport(content), [content]);
  if (compact) {
    return (
      <CompactToolDetails facts={finalReportPresentation(content).facts}>
        <div className={styles.fullReport}>
          <FinalReportView content={content} title={title} />
        </div>
      </CompactToolDetails>
    );
  }
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
        <Badge tone={report.success ? "muted" : "danger"}>
          <Icon
            icon={report.success ? CheckCircle2 : CircleX}
            size="sm"
            tone={report.success ? "muted" : "danger"}
          />
          {report.success ? "Reported success" : "Reported failure"}
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
        <p className={styles.none}>{verificationSummary(report)}</p>
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
      <ToolCard title="Technical details" defaultOpen={false}>
        <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
      </ToolCard>
    </div>
  );
};

export default FinalReportView;
