import { type ReactNode, useState } from "react";
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
import { Badge, Icon } from "../ui";
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
    <div className={styles.card}>
      <button
        type="button"
        className={styles.header}
        aria-expanded={expanded}
        aria-controls="buddy-pulse-sections"
        onClick={() => setExpanded((x) => !x)}
      >
        <h3 className={styles.headerTitle}>
          <Icon icon={Sparkles} size="sm" tone="accent" /> Project pulse ·
          updated {minutesAgo}m ago
        </h3>
      </button>
      {expanded && (
        <div id="buddy-pulse-sections" className={styles.sections}>
          <Section
            icon={SECTION_ICONS.preferences}
            title="Preferences"
            count={payload.preferences.length}
          >
            {payload.preferences.map((preference) => (
              <p key={preference.statement} className={styles.line}>
                {preference.statement}{" "}
                <Badge tone="muted">
                  conf {preference.confidence.toFixed(2)}
                </Badge>
              </p>
            ))}
          </Section>
          <Section
            icon={SECTION_ICONS.lessons}
            title="Lessons"
            count={payload.lessons.length}
          >
            {payload.lessons.map((lesson) => (
              <p key={lesson.title} className={styles.line}>
                <strong>{lesson.title}</strong> — {lesson.preview}
              </p>
            ))}
          </Section>
          <Section
            icon={SECTION_ICONS.friction}
            title="Friction"
            count={payload.friction.top_error_types.length}
          >
            <p className={styles.line}>
              Stuck tasks: {payload.friction.stuck_tasks}
            </p>
            {payload.friction.top_error_types.map((error) => (
              <p key={error.type} className={styles.line}>
                {error.type}: {error.count}
              </p>
            ))}
          </Section>
          <Section
            icon={SECTION_ICONS.reports}
            title="Recent reports"
            count={payload.recent_reports.length}
          >
            {payload.recent_reports.map((report) => (
              <p key={report.chat_id} className={styles.line}>
                <strong>{report.title}</strong> — {report.preview}
              </p>
            ))}
          </Section>
          <Section
            icon={SECTION_ICONS.activity}
            title="Activity (24h)"
            count={payload.user_activity.grouped.length}
          >
            <p className={styles.line}>
              {payload.user_activity.time_of_day_pattern}
            </p>
            {payload.user_activity.grouped.map((group) => (
              <p key={group.type} className={styles.line}>
                {group.type}: {group.count}
              </p>
            ))}
          </Section>
        </div>
      )}
    </div>
  );
};

const Section = ({ icon, title, count, children }: SectionProps) => (
  <section className={styles.section}>
    <h4 className={styles.sectionTitle}>
      <Icon icon={icon} size="sm" tone="muted" /> {title}{" "}
      <Badge tone="muted">{count}</Badge>
    </h4>
    <div className={styles.sectionBody}>{children}</div>
  </section>
);

type SectionProps = {
  icon: LucideIcon;
  title: string;
  count: number;
  children: ReactNode;
};
