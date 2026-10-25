import type {
  PendingQuestion,
  QuestionResponseItem,
} from "../services/refact/types";

export type QuestionDraft = {
  mode: "standard" | "custom";
  value?: string | string[];
  custom: string;
  explanation: string;
  skipped: boolean;
};

export const emptyQuestionDraft = (): QuestionDraft => ({
  mode: "standard",
  custom: "",
  explanation: "",
  skipped: false,
});

export function questionResponse(
  question: PendingQuestion,
  draft: QuestionDraft,
): QuestionResponseItem | null {
  if (draft.skipped)
    return {
      id: question.id,
      disposition: "skipped",
      explanation: draft.explanation,
    };
  if (draft.mode === "custom") {
    return draft.custom.trim()
      ? {
          id: question.id,
          disposition: "custom",
          answer: draft.custom,
          explanation: draft.explanation,
        }
      : null;
  }
  const value = draft.value;
  if (question.type === "multi_select") {
    return Array.isArray(value)
      ? {
          id: question.id,
          disposition: "answered",
          answer: value,
          explanation: draft.explanation,
        }
      : null;
  }
  if (typeof value !== "string" || !value.trim()) return null;
  return {
    id: question.id,
    disposition: "answered",
    answer: question.type === "yes_no" ? value === "Yes" : value,
    explanation: draft.explanation,
  };
}

export function responseLabel(item: QuestionResponseItem): string {
  if (item.disposition === "skipped") return "Skipped";
  const value = item.answer;
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (Array.isArray(value))
    return value.length ? value.join(", ") : "None of these";
  return value;
}
