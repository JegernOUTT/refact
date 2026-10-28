import React from "react";
import type {
  AgentQuestion,
  BackgroundAgentSummary,
  HandoffGap,
} from "../../services/refact/types";
import styles from "./BackgroundAgentCard.module.css";

const MAX_HANDOFF_ITEMS = 10;
const MAX_HANDOFF_CHARS = 300;

function handoffItem(gap: HandoffGap): string {
  const detail = gap.detail.trim() || gap.category.replace(/_/g, " ");
  const item = gap.path ? `${detail} (${gap.path})` : detail;
  const characters = Array.from(item);
  return characters.length > MAX_HANDOFF_CHARS
    ? `${characters.slice(0, MAX_HANDOFF_CHARS - 1).join("")}…`
    : item;
}

const GUESS_CATEGORIES = new Set(["missing_decision", "ambiguous_requirement"]);

export const HandoffBlock: React.FC<{ agent: BackgroundAgentSummary }> = ({
  agent,
}) => {
  const gaps = agent.handoff?.gaps ?? [];
  const groups = [
    {
      label: "Had to discover",
      items: gaps.filter((gap) => !GUESS_CATEGORIES.has(gap.category)),
    },
    {
      label: "Had to guess",
      items: gaps.filter((gap) => GUESS_CATEGORIES.has(gap.category)),
    },
  ];
  return (
    <section aria-label="Handoff" className={styles.block}>
      <h5 className={styles.blockTitle}>Handoff</h5>
      {gaps.length === 0 ? (
        <p className={styles.muted}>Nothing declared</p>
      ) : (
        groups.map(({ label, items }) =>
          items.length === 0 ? null : (
            <div className={styles.block} key={label}>
              <h6 className={styles.groupTitle}>{label}</h6>
              <ul className={styles.list}>
                {items.slice(0, MAX_HANDOFF_ITEMS).map((gap, index) => (
                  <li key={`${gap.category}-${index}`}>{handoffItem(gap)}</li>
                ))}
              </ul>
              {items.length > MAX_HANDOFF_ITEMS && (
                <span className={styles.muted}>
                  +{items.length - MAX_HANDOFF_ITEMS} more
                </span>
              )}
            </div>
          ),
        )
      )}
    </section>
  );
};

export const QuestionsBlock: React.FC<{
  questions: readonly AgentQuestion[];
}> = ({ questions }) =>
  questions.length === 0 ? null : (
    <section aria-label="Agent questions and answers" className={styles.block}>
      <h5 className={styles.blockTitle}>Q&amp;A</h5>
      <ul className={styles.list}>
        {questions.map((question) => (
          <li className={styles.question} key={question.id}>
            <span>Q: {question.text}</span>
            <span className={styles.muted}>
              {question.answer ? `A: ${question.answer}` : "Awaiting reply"}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
