import { useId, useState } from "react";
import { Button, FieldTextarea, SegmentedControl, Switch } from "../../ui";
import { Markdown } from "../../Markdown";
import { QuestionArtifactContext } from "../../Markdown/questionArtifactContext";
import type {
  PendingQuestion,
  QuestionResponse,
} from "../../../services/refact/types";
import {
  emptyQuestionDraft,
  questionResponse,
  type QuestionDraft,
} from "../../../utils/questionResponses";
import styles from "./AskQuestionsTool.module.css";

type Props = {
  questions: PendingQuestion[];
  instance: string;
  drafts: Partial<Record<string, QuestionDraft>>;
  comments: string;
  onDrafts: (drafts: Partial<Record<string, QuestionDraft>>) => void;
  onComments: (comments: string) => void;
  onSubmit: (response: QuestionResponse) => void;
  disabled: boolean;
  submitting?: boolean;
  approval?: boolean;
};

export function QuestionResponseForm({
  questions,
  instance,
  drafts,
  comments,
  onDrafts,
  onComments,
  onSubmit,
  disabled,
  submitting = false,
  approval = false,
}: Props) {
  const responses = questions.map((q) =>
    questionResponse(q, drafts[q.id] ?? emptyQuestionDraft()),
  );
  const skipped = responses.filter((r) => r?.disposition === "skipped").length;
  const answered = responses.filter(
    (r) => r && r.disposition !== "skipped",
  ).length;
  const remaining = questions.length - answered - skipped;
  const update = (id: string, patch: Partial<QuestionDraft>) => {
    if (!disabled)
      onDrafts({
        ...drafts,
        [id]: { ...(drafts[id] ?? emptyQuestionDraft()), ...patch },
      });
  };
  return (
    <div className={styles.responseForm}>
      <p className={styles.muted}>
        {approval
          ? "Only an explicit Approve grants permission. Skipping or writing a custom response declines this permission request and passes your context to the agent."
          : "Choose, write your own answer, or skip. Nothing is a forced choice."}
      </p>
      <div role="status" aria-live="polite" className={styles.progress}>
        {answered} answered · {skipped} skipped · {remaining} remaining
      </div>
      {questions.map((question, index) => (
        <ResponseQuestion
          key={question.id}
          question={question}
          index={index}
          instance={instance}
          draft={drafts[question.id] ?? emptyQuestionDraft()}
          disabled={disabled}
          onChange={(patch) => update(question.id, patch)}
        />
      ))}
      <label className={styles.comments}>
        <span>
          Overall comments <span className={styles.muted}>(optional)</span>
        </span>
        <FieldTextarea
          value={comments}
          onChange={onComments}
          disabled={disabled}
          maxLength={8192}
          placeholder="Anything that applies to the whole request…"
        />
      </label>
      <div className={styles.formActions}>
        <Button
          size="sm"
          disabled={disabled || !remaining}
          onClick={() => {
            const next = { ...drafts };
            questions.forEach((q, i) => {
              if (!responses[i])
                next[q.id] = {
                  ...(drafts[q.id] ?? emptyQuestionDraft()),
                  skipped: true,
                };
            });
            onDrafts(next);
          }}
        >
          Skip remaining
        </Button>
        <Button
          variant="primary"
          disabled={disabled || remaining > 0}
          onClick={() => {
            if (remaining || disabled) return;
            onSubmit({
              answers: responses.filter(
                (r): r is NonNullable<typeof r> => r !== null,
              ),
              overall_comments: comments,
            });
          }}
        >
          {submitting ? "Submitting…" : "Submit response"}
        </Button>
      </div>
      <p className={styles.muted}>
        {remaining
          ? "Answer or skip each remaining question."
          : approval
            ? "Skipping declines this permission request. Custom text never grants permission."
            : "Skipped questions are passed to the agent explicitly—not treated as No or approval."}
      </p>
    </div>
  );
}

function ResponseQuestion({
  question,
  index,
  instance,
  draft,
  disabled,
  onChange,
}: {
  question: PendingQuestion;
  index: number;
  instance: string;
  draft: QuestionDraft;
  disabled: boolean;
  onChange: (patch: Partial<QuestionDraft>) => void;
}) {
  const id = useId();
  const [showExplanation, setShowExplanation] = useState(
    Boolean(draft.explanation),
  );
  const response = questionResponse(question, draft);
  const value = draft.value ?? (question.type === "multi_select" ? [] : "");
  const onValue = (value: string | string[]) => onChange({ value });
  return (
    <section
      className={styles.responseQuestion}
      aria-labelledby={`${id}-prompt`}
    >
      <div className={styles.questionHeader}>
        <span className={styles.muted}>Question {index + 1}</span>
        <Button
          size="sm"
          disabled={disabled}
          onClick={() => onChange({ skipped: !draft.skipped })}
        >
          {draft.skipped ? "Undo skip" : "Skip"}
        </Button>
      </div>
      <div id={`${id}-prompt`}>
        <QuestionArtifactContext.Provider
          value={{
            instance: `${instance}:${question.id}`,
            enabled: !disabled && !draft.skipped,
            onAnswer: (answer) => onChange({ mode: "custom", custom: answer }),
          }}
        >
          <Markdown>{question.text}</Markdown>
        </QuestionArtifactContext.Provider>
      </div>
      {draft.skipped ? (
        <p className={styles.muted}>
          Skipped. The agent will decide how to proceed. Undo keeps your draft.
        </p>
      ) : (
        <>
          <div className={styles.questionHeader}>
            <span className={styles.muted}>
              {response
                ? draft.mode === "custom"
                  ? "Custom answer ready"
                  : "Answer ready"
                : "Not answered"}
            </span>
            {question.type !== "free_text" && (
              <Button
                size="sm"
                disabled={disabled}
                onClick={() =>
                  onChange({
                    mode: draft.mode === "custom" ? "standard" : "custom",
                  })
                }
              >
                {draft.mode === "custom"
                  ? "Back to choices"
                  : "Write my own answer"}
              </Button>
            )}
          </div>
          {draft.mode === "custom" ? (
            <FieldTextarea
              aria-label={`Your answer to question ${index + 1}`}
              value={draft.custom}
              onChange={(custom) => onChange({ custom })}
              disabled={disabled}
              maxLength={8192}
              placeholder="Tell the agent what you actually want…"
            />
          ) : question.type === "yes_no" ? (
            <SegmentedControl
              aria-label={`Answer to question ${index + 1}`}
              name={id}
              options={[
                { value: "Yes", label: "Yes", disabled },
                { value: "No", label: "No", disabled },
              ]}
              value={typeof value === "string" ? value : ""}
              onValueChange={onValue}
            />
          ) : question.type === "single_select" ? (
            <div
              role="radiogroup"
              aria-labelledby={`${id}-prompt`}
              className={styles.singleSelectGroup}
            >
              {question.options?.map((option) => (
                <label
                  key={option}
                  className={`${styles.singleSelectOption} ${
                    value === option ? styles.singleSelectOptionSelected : ""
                  }`}
                >
                  <input
                    className={styles.singleSelectInput}
                    type="radio"
                    name={id}
                    checked={value === option}
                    disabled={disabled}
                    onChange={() => onValue(option)}
                  />
                  <span className={styles.singleSelectIndicator} />
                  <span className={styles.singleSelectLabel}>{option}</span>
                </label>
              ))}
            </div>
          ) : question.type === "multi_select" ? (
            <div
              className={styles.multiSelect}
              role="group"
              aria-labelledby={`${id}-prompt`}
            >
              {question.options?.map((option) => (
                <Switch
                  key={option}
                  label={option}
                  disabled={disabled}
                  checked={Array.isArray(value) && value.includes(option)}
                  onCheckedChange={(checked) => {
                    const selected = Array.isArray(value) ? value : [];
                    onValue(
                      checked
                        ? [...selected, option]
                        : selected.filter((v) => v !== option),
                    );
                  }}
                />
              ))}
              <Button size="sm" disabled={disabled} onClick={() => onValue([])}>
                None of these
              </Button>
            </div>
          ) : (
            <FieldTextarea
              aria-label={`Your answer to question ${index + 1}`}
              value={typeof value === "string" ? value : ""}
              onChange={onValue}
              disabled={disabled}
              maxLength={8192}
              placeholder="Type your answer…"
            />
          )}
          <Button
            size="sm"
            disabled={disabled}
            aria-expanded={showExplanation}
            onClick={() => setShowExplanation(!showExplanation)}
          >
            {showExplanation ? "Hide explanation" : "Add explanation"}
          </Button>
          {showExplanation && (
            <FieldTextarea
              aria-label={`Explanation for question ${index + 1}`}
              value={draft.explanation}
              onChange={(explanation) => onChange({ explanation })}
              disabled={disabled}
              maxLength={8192}
              placeholder="Optional context for this answer…"
            />
          )}
        </>
      )}
    </section>
  );
}
