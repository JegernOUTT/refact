import React, { useMemo } from "react";
import { Badge, Box, Flex, Text } from "@radix-ui/themes";
import { CheckCircle2, CircleX } from "lucide-react";
import { Markdown, ShikiCodeBlock } from "../Markdown";
import { Icon } from "../ui";
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
    )
      return null;
    if (
      exitCode !== undefined &&
      exitCode !== null &&
      typeof exitCode !== "number"
    )
      return null;
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
    )
      return null;
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
  )
    return null;
  const files = optionalStringArray(raw.files_changed);
  const tests = optionalStringArray(raw.tests_added_or_updated);
  const verification = parseVerification(raw.verification);
  const followups = parseFollowups(raw.followup_cards);
  const risks = optionalStringArray(raw.risks);
  const assumptions = optionalStringArray(raw.assumptions);
  if (!files || !tests || !verification || !followups || !risks || !assumptions)
    return null;
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
  <Box className={styles.section}>
    <Text as="div" size="2" weight="medium" className={styles.sectionTitle}>
      {title}
    </Text>
    {children}
  </Box>
);

const TextDetails: React.FC<{ title: string; items: string[] }> = ({
  title,
  items,
}) =>
  items.length > 0 ? (
    <details className={styles.details}>
      <summary>
        {title} ({items.length})
      </summary>
      <ul className={styles.list}>
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    </details>
  ) : (
    <Section title={title}>
      <Text size="2" color="gray">
        None
      </Text>
    </Section>
  );

export const FinalReportView: React.FC<FinalReportViewProps> = ({
  content,
  title = "Final Report",
}) => {
  const report = useMemo(() => parseFinalReport(content), [content]);
  if (!report)
    return (
      <Box className={styles.legacy}>
        <Markdown>{content}</Markdown>
      </Box>
    );

  return (
    <Box className={styles.root} data-testid="final-report-view">
      <Flex justify="between" align="center" gap="2" className={styles.header}>
        <Text weight="medium" className={styles.title}>
          {title}
        </Text>
        <Badge color={report.success ? "green" : "red"} variant="soft">
          <Icon
            icon={report.success ? CheckCircle2 : CircleX}
            size="sm"
            tone={report.success ? "success" : "danger"}
          />
          {report.success ? "Success" : "Failed"}
        </Badge>
      </Flex>
      <Section title="Summary">
        <Box className={styles.markdown}>
          <Markdown>{report.summary}</Markdown>
        </Box>
      </Section>
      <Section title="Files changed">
        {report.files_changed.length > 0 ? (
          <Flex gap="2" wrap="wrap">
            {report.files_changed.map((file) => (
              <Badge key={file} variant="surface" color="gray">
                {file}
              </Badge>
            ))}
          </Flex>
        ) : (
          <Text size="2" color="gray">
            None
          </Text>
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
          <Text size="2" color="gray">
            None
          </Text>
        )}
      </Section>
      <Section title="Verification">
        {report.verification.length > 0 ? (
          <Flex direction="column" gap="2">
            {report.verification.map((item) => (
              <details
                key={`${item.command}:${item.exit_code ?? ""}`}
                className={styles.verificationItem}
              >
                <summary className={styles.verificationHeader}>
                  <Icon
                    icon={item.passed ? CheckCircle2 : CircleX}
                    size="sm"
                    tone={item.passed ? "success" : "danger"}
                  />
                  <code>{item.command}</code>
                  {item.exit_code !== undefined && item.exit_code !== null && (
                    <Text as="span" size="1" color="gray">
                      ({item.exit_code})
                    </Text>
                  )}
                </summary>
                {item.output_tail && (
                  <ShikiCodeBlock showLineNumbers={false}>
                    {item.output_tail}
                  </ShikiCodeBlock>
                )}
              </details>
            ))}
          </Flex>
        ) : (
          <Text size="2" color="gray">
            None
          </Text>
        )}
      </Section>
      <Section title="Followup cards">
        {report.followup_cards.length > 0 ? (
          <Flex direction="column" gap="2">
            {report.followup_cards.map((card) => (
              <Box key={card.title} className={styles.followupCard}>
                <Flex gap="2" align="center">
                  <Text size="2" weight="medium">
                    {card.title}
                  </Text>
                  <Badge variant="soft" color="gray">
                    {card.priority}
                  </Badge>
                </Flex>
                <Text as="p" size="2" color="gray">
                  {truncate(card.instructions)}
                </Text>
              </Box>
            ))}
          </Flex>
        ) : (
          <Text size="2" color="gray">
            None
          </Text>
        )}
      </Section>
      <TextDetails title="Risks" items={report.risks} />
      <TextDetails title="Assumptions" items={report.assumptions} />
    </Box>
  );
};

export default FinalReportView;
