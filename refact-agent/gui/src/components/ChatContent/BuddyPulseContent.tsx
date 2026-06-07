import { type ReactNode, useState } from "react";
import { Badge, Box, Card, Flex, Heading, Text } from "@radix-ui/themes";
import type { LucideIcon } from "lucide-react";
import {
  BookOpen,
  Compass,
  MousePointerClick,
  Search,
  Sparkles,
  TriangleAlert,
} from "lucide-react";
import {
  type BuddyPulsePayload,
  isBuddyPulsePayload,
} from "../../services/refact/types";
import { Icon } from "../ui";
import styles from "./BuddyPulseContent.module.css";

type Props = {
  rawExtra: unknown;
};

const SECTION_ICONS = {
  preferences: Compass,
  lessons: BookOpen,
  friction: TriangleAlert,
  reports: Search,
  activity: MousePointerClick,
} as const;

export const BuddyPulseContent = ({ rawExtra }: Props) => {
  const [expanded, setExpanded] = useState(false);

  const payload =
    rawExtra &&
    isBuddyPulsePayload(
      (rawExtra as Record<string, unknown>).buddy_pulse_payload,
    )
      ? ((rawExtra as Record<string, unknown>)
          .buddy_pulse_payload as BuddyPulsePayload)
      : null;

  if (!payload) return null;

  const generated = new Date(payload.generated_at);
  const minutesAgo = Math.floor((Date.now() - generated.getTime()) / 60_000);

  return (
    <Card className={styles.card}>
      <button
        type="button"
        className={styles.header}
        aria-expanded={expanded}
        aria-controls="buddy-pulse-sections"
        onClick={() => setExpanded((x) => !x)}
      >
        <Heading size="3" className={styles.headerTitle}>
          <Icon icon={Sparkles} size="sm" tone="accent" /> Project pulse · updated{" "}
          {minutesAgo}m ago
        </Heading>
      </button>
      {expanded && (
        <Box id="buddy-pulse-sections" className={styles.sections}>
          <Section
            icon={SECTION_ICONS.preferences}
            title="Preferences"
            count={payload.preferences.length}
          >
            {payload.preferences.map((preference) => (
              <Text key={preference.statement} as="p" size="2">
                {preference.statement}{" "}
                <Badge color="gray">
                  conf {preference.confidence.toFixed(2)}
                </Badge>
              </Text>
            ))}
          </Section>
          <Section
            icon={SECTION_ICONS.lessons}
            title="Lessons"
            count={payload.lessons.length}
          >
            {payload.lessons.map((lesson) => (
              <Text key={lesson.title} as="p" size="2">
                <strong>{lesson.title}</strong> — {lesson.preview}
              </Text>
            ))}
          </Section>
          <Section
            icon={SECTION_ICONS.friction}
            title="Friction"
            count={payload.friction.top_error_types.length}
          >
            <Text as="p" size="2">
              Stuck tasks: {payload.friction.stuck_tasks}
            </Text>
            {payload.friction.top_error_types.map((error) => (
              <Text key={error.type} as="p" size="2">
                {error.type}: {error.count}
              </Text>
            ))}
          </Section>
          <Section
            icon={SECTION_ICONS.reports}
            title="Recent reports"
            count={payload.recent_reports.length}
          >
            {payload.recent_reports.map((report) => (
              <Text key={report.chat_id} as="p" size="2">
                <strong>{report.title}</strong> — {report.preview}
              </Text>
            ))}
          </Section>
          <Section
            icon={SECTION_ICONS.activity}
            title="Activity (24h)"
            count={payload.user_activity.grouped.length}
          >
            <Text as="p" size="2">
              {payload.user_activity.time_of_day_pattern}
            </Text>
            {payload.user_activity.grouped.map((group) => (
              <Text key={group.type} as="p" size="2">
                {group.type}: {group.count}
              </Text>
            ))}
          </Section>
        </Box>
      )}
    </Card>
  );
};

const Section = ({ icon, title, count, children }: SectionProps) => (
  <Flex direction="column" gap="1" className={styles.section}>
    <Heading size="2" className={styles.sectionTitle}>
      <Icon icon={icon} size="sm" /> {title}{" "}
      <Badge color="gray">{count}</Badge>
    </Heading>
    <Box>{children}</Box>
  </Flex>
);

type SectionProps = {
  icon: LucideIcon;
  title: string;
  count: number;
  children: ReactNode;
};
