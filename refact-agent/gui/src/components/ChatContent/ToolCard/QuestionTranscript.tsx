import React, { useState } from "react";
import { CircleCheck, CircleHelp, CircleMinus } from "lucide-react";
import { Markdown } from "../../Markdown";
import { QuestionArtifactContext } from "../../Markdown/questionArtifactContext";
import type { PendingQuestion, ToolCall } from "../../../services/refact/types";
import { compactText } from "../../Tools/AgenticTool/compactPresentation";
import {
  TranscriptFacts,
  TranscriptItem,
  type TranscriptFact,
  type TranscriptStatus,
  type UnrecordedResultState,
} from "../TranscriptItem";
import styles from "./AskQuestionsTool.module.css";

export type QuestionState =
  | "pending"
  | "sent"
  | "send_failed"
  | "answered"
  | "cancelled"
  | "syncing"
  | "failed"
  | "unknown";

type QuestionLike = Pick<PendingQuestion, "id" | "text"> & {
  options?: string[];
};

interface StateView {
  status: TranscriptStatus;
  statusLabel?: string;
  meta?: string;
  tone: "running" | "success" | "error" | "idle";
}

const TARGET_CHARS = 120;

function stateView(state: QuestionState, count: number): StateView {
  const many = count > 1 ? `${count} questions` : undefined;
  switch (state) {
    case "pending":
      return { status: "needs_you", meta: many, tone: "running" };
    case "sent":
      return {
        status: "waiting",
        statusLabel: "Awaiting confirmation",
        meta: many,
        tone: "running",
      };
    case "send_failed":
      return {
        status: "needs_you",
        meta: "send failed",
        tone: "error",
      };
    case "answered":
      return { status: "done", meta: "answered", tone: "success" };
    case "cancelled":
      return { status: "cancelled", statusLabel: "Withdrawn", tone: "idle" };
    case "syncing":
      return { status: "waiting", statusLabel: "Syncing", tone: "running" };
    case "failed":
      return {
        status: "failed",
        statusLabel: "Unavailable",
        tone: "error",
      };
    default:
      return { status: "unknown", statusLabel: "State unknown", tone: "idle" };
  }
}

function isLive(state: QuestionState): boolean {
  return state === "pending" || state === "sent" || state === "send_failed";
}

export function QuestionList({
  questions,
  instance,
}: {
  questions: readonly QuestionLike[];
  instance: string;
}) {
  return (
    <ol className={styles.questionList} data-testid="question-list">
      {questions.map((question) => (
        <li className={styles.questionEntry} key={question.id}>
          <QuestionArtifactContext.Provider
            value={{
              instance: `${instance}:${question.id}`,
              enabled: false,
              onAnswer: () => undefined,
            }}
          >
            <Markdown>{question.text}</Markdown>
          </QuestionArtifactContext.Provider>
          {question.options?.length ? (
            <ul className={styles.optionList}>
              {question.options.map((option) => (
                <li key={option}>{option}</li>
              ))}
            </ul>
          ) : null}
        </li>
      ))}
    </ol>
  );
}

export interface AnswerEntry {
  id: string;
  question: string;
  answer: string;
  explanation?: string;
}

export function AnswerList({
  answers,
  comments,
}: {
  answers: readonly AnswerEntry[];
  comments?: string;
}) {
  const facts: TranscriptFact[] = answers.map((entry) => ({
    label: compactText(entry.question, TARGET_CHARS) ?? entry.id,
    title: entry.question,
    value: (
      <span className={styles.answerValue}>
        <span>{entry.answer}</span>
        {entry.explanation ? (
          <span className={styles.answerText}>{entry.explanation}</span>
        ) : null}
      </span>
    ),
  }));
  if (comments) facts.push({ label: "Overall comments", value: comments });
  return <TranscriptFacts facts={facts} />;
}

export interface QuestionTranscriptProps {
  toolCall: ToolCall;
  state: QuestionState;
  questions: readonly QuestionLike[];
  input?: React.ReactNode;
  output?: React.ReactNode;
  error?: React.ReactNode;
  actions?: React.ReactNode;
  unrecorded?: UnrecordedResultState;
}

export function QuestionTranscript({
  toolCall,
  state,
  questions,
  input,
  output,
  error,
  actions,
  unrecorded,
}: QuestionTranscriptProps) {
  const [liveOpen, setLiveOpen] = useState(true);
  const view = stateView(state, questions.length);
  const live = isLive(state);
  const icon =
    state === "answered" ? (
      <CircleCheck />
    ) : state === "cancelled" ? (
      <CircleMinus />
    ) : (
      <CircleHelp />
    );
  const openProps = live
    ? { open: liveOpen, onOpenChange: setLiveOpen }
    : { openKey: toolCall.id ? `tc:${toolCall.id}` : undefined };

  return (
    <TranscriptItem
      actions={actions}
      className="rf-enter"
      data-status={view.tone}
      data-testid="ask-questions-tool"
      error={error}
      icon={icon}
      input={input}
      meta={view.meta}
      output={output}
      raw={[{ label: "Questions", value: questions }]}
      status={unrecorded?.status ?? view.status}
      statusLabel={unrecorded ? unrecorded.statusLabel : view.statusLabel}
      target={compactText(questions[0]?.text, TARGET_CHARS) ?? "Question"}
      verb="Question"
      {...openProps}
    />
  );
}
