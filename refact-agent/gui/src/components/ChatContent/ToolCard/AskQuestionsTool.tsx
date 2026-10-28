import React, {
  useMemo,
  useState,
  useCallback,
  useEffect,
  useRef,
} from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
import { Button, FieldTextarea, SegmentedControl, Switch } from "../../ui";
import { Markdown } from "../../Markdown";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { useAppDispatch } from "../../../hooks/useAppDispatch";
import { useChatActions } from "../../../hooks/useChatActions";
import {
  selectToolResultByThreadAndId,
  selectMessagesById,
  selectPendingQuestionSetsById,
  selectHasPendingQuestionsCapabilityById,
  selectCommandAckByThreadAndId,
  selectCommandCancelledByThreadAndId,
} from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import { requestSseRefresh } from "../../../features/Chat/Thread/actions";
import {
  selectApiKey,
  selectConfig,
} from "../../../features/Config/configSlice";
import { answerQuestions } from "../../../services/refact/chatCommands";
import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import {
  ToolCall,
  PendingQuestion,
  PendingQuestionAnswer,
  isUserMessage,
  isToolMessage,
} from "../../../services/refact/types";
import {
  clearAskQuestionsDraft,
  loadAskQuestionsDraft,
  saveAskQuestionsDraft,
  type AskQuestionsDraftValue,
} from "../../../utils/chatUiPersistence";
import {
  AnswerList,
  QuestionList,
  QuestionTranscript,
  type AnswerEntry,
  type QuestionState,
} from "./QuestionTranscript";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";
import styles from "./AskQuestionsTool.module.css";
import { NativeQuestionResponse } from "./NativeQuestionResponse";

type QuestionItem = PendingQuestion;

interface AskQuestionsResult {
  type: "ask_questions";
  tool_call_id: string;
  questions: QuestionItem[];
}

interface AskQuestionsToolProps {
  toolCall: ToolCall;
  compact?: boolean;
}

function formatAnswers(
  marker: string,
  questions: QuestionItem[],
  answers: Record<string, string | string[]>,
  additional: string,
): string {
  const lines = [marker];

  for (const q of questions) {
    const answer = answers[q.id];
    lines.push(`> [${q.id}] ${q.text}`);
    if (Array.isArray(answer)) {
      lines.push(answer.length > 0 ? answer.join(", ") : "(no selection)");
    } else if (answer && answer.includes("\n")) {
      lines.push("```");
      lines.push(answer);
      lines.push("```");
    } else {
      lines.push(answer || "(no answer)");
    }
    lines.push("");
  }

  if (additional.trim()) {
    lines.push("> [__additional__] Additional comments");
    if (additional.includes("\n")) {
      lines.push("```");
      lines.push(additional.trim());
      lines.push("```");
    } else {
      lines.push(additional.trim());
    }
  }

  return lines.join("\n").trim();
}

function parseAnswersFromMessage(
  content: string,
  questions: QuestionItem[],
): Record<string, string> | null {
  const result: Record<string, string> = {};
  const idSet = new Set(questions.map((q) => q.id));
  idSet.add("__additional__");

  const regex = /^> \[([^\]]+)\]/gm;
  let match;
  const positions: { id: string; start: number }[] = [];

  while ((match = regex.exec(content)) !== null) {
    if (idSet.has(match[1])) {
      positions.push({ id: match[1], start: match.index });
    }
  }

  for (let i = 0; i < positions.length; i++) {
    const { id, start } = positions[i];
    const lineEnd = content.indexOf("\n", start);
    if (lineEnd === -1) continue; // Guard against missing newline
    const answerStart = lineEnd + 1;
    const answerEnd =
      i + 1 < positions.length ? positions[i + 1].start : content.length;

    let answer = content.slice(answerStart, answerEnd).trim();
    if (answer.startsWith("```") && answer.includes("```", 3)) {
      const codeStart = answer.indexOf("\n") + 1;
      const codeEnd = answer.lastIndexOf("```");
      answer = answer.slice(codeStart, codeEnd).trim();
    }
    if (answer) {
      result[id] = answer;
    }
  }

  return Object.keys(result).length > 0 ? result : null;
}

const QuestionWidget: React.FC<{
  question: QuestionItem;
  value: string | string[];
  onChange: (val: string | string[]) => void;
  disabled?: boolean;
}> = ({ question, value, onChange, disabled = false }) => {
  switch (question.type) {
    case "yes_no":
      return (
        <Box className={styles.questionItem}>
          <Box mb="2">
            <Markdown>{question.text}</Markdown>
          </Box>
          <SegmentedControl
            name={question.id}
            options={[
              { value: "Yes", label: "Yes", disabled },
              { value: "No", label: "No", disabled },
            ]}
            value={typeof value === "string" ? value : ""}
            onValueChange={(nextValue) => {
              if (!disabled) onChange(nextValue);
            }}
          />
        </Box>
      );

    case "single_select":
      return (
        <Box className={styles.questionItem}>
          <Box mb="2">
            <Markdown>{question.text}</Markdown>
          </Box>
          <Flex
            aria-label={question.text}
            className={styles.singleSelectGroup}
            direction="column"
            gap="2"
            role="radiogroup"
          >
            {question.options?.map((opt) => {
              const checked = typeof value === "string" && value === opt;
              const optionClassName = checked
                ? `${styles.singleSelectOption} ${styles.singleSelectOptionSelected}`
                : styles.singleSelectOption;
              return (
                <label key={opt} className={optionClassName}>
                  <input
                    className={styles.singleSelectInput}
                    type="radio"
                    name={question.id}
                    value={opt}
                    checked={checked}
                    disabled={disabled}
                    onChange={() => onChange(opt)}
                  />
                  <span className={styles.singleSelectIndicator} />
                  <span className={styles.singleSelectLabel}>{opt}</span>
                </label>
              );
            })}
          </Flex>
        </Box>
      );

    case "multi_select":
      return (
        <Box className={styles.questionItem}>
          <Box mb="2">
            <Markdown>{question.text}</Markdown>
          </Box>
          <Flex direction="column" gap="2">
            {question.options?.map((opt) => {
              const checked = Array.isArray(value) && value.includes(opt);
              return (
                <Switch
                  key={opt}
                  checked={checked}
                  disabled={disabled}
                  label={opt}
                  onCheckedChange={(nextChecked) => {
                    const current = Array.isArray(value) ? value : [];
                    if (nextChecked) {
                      onChange([...current, opt]);
                      return;
                    }
                    onChange(current.filter((v) => v !== opt));
                  }}
                />
              );
            })}
          </Flex>
        </Box>
      );

    case "free_text":
      return (
        <Box className={styles.questionItem}>
          <Box mb="2">
            <Markdown>{question.text}</Markdown>
          </Box>
          <FieldTextarea
            value={typeof value === "string" ? value : ""}
            onChange={onChange}
            disabled={disabled}
            placeholder="Type your answer..."
          />
        </Box>
      );

    default:
      return null;
  }
};

export const AskQuestionsTool: React.FC<AskQuestionsToolProps> = ({
  toolCall,
}) => {
  const [answers, setAnswers] = useState<
    Record<string, AskQuestionsDraftValue>
  >(() => loadAskQuestionsDraft(toolCall.id)?.answers ?? {});
  const [additionalText, setAdditionalText] = useState(
    () => loadAskQuestionsDraft(toolCall.id)?.additionalText ?? "",
  );
  const threadId = useThreadId();
  const { submit } = useChatActions(threadId);
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [submissionSent, setSubmissionSent] = useState(false);
  const [submissionRequestId, setSubmissionRequestId] = useState<string | null>(
    null,
  );
  const [submitError, setSubmitError] = useState<string | null>(null);

  const unrecorded = useUnrecordedResultState(toolCall.id);
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );

  const messages = useAppSelector((state) =>
    selectMessagesById(state, threadId),
  );
  const pendingQuestionSets = useAppSelector((state) =>
    selectPendingQuestionSetsById(state, threadId),
  );
  const hasNativeQuestionCapability = useAppSelector((state) =>
    selectHasPendingQuestionsCapabilityById(state, threadId),
  );
  const submissionAck = useAppSelector((state) =>
    selectCommandAckByThreadAndId(state, threadId, submissionRequestId),
  );
  const submissionCancelled = useAppSelector((state) =>
    selectCommandCancelledByThreadAndId(state, threadId, submissionRequestId),
  );

  const nativeQuestionSet = useMemo(
    () => pendingQuestionSets.find((set) => set.tool_call_id === toolCall.id),
    [pendingQuestionSets, toolCall.id],
  );
  const questionInstance = JSON.stringify([
    getEngineEndpointIdentity(config),
    threadId,
    toolCall.id,
    nativeQuestionSet?.asked_at_ms,
  ]);
  const currentQuestionInstance = useRef(questionInstance);
  currentQuestionInstance.current = questionInstance;
  const submissionInstance = useRef<string | null>(null);

  useEffect(() => {
    setIsSubmitting(false);
    setSubmissionSent(false);
    setSubmissionRequestId(null);
    setSubmitError(null);
  }, [questionInstance]);

  const data = useMemo((): AskQuestionsResult | null => {
    if (!maybeResult || typeof maybeResult.content !== "string") return null;
    try {
      return JSON.parse(maybeResult.content) as AskQuestionsResult;
    } catch {
      return null;
    }
  }, [maybeResult]);

  const marker = `[QA:${toolCall.id}]`;

  const nextUserMessage = useMemo(() => {
    if (!maybeResult) return null;

    let foundToolResult = false;
    for (const msg of messages) {
      if (isToolMessage(msg) && msg.tool_call_id === toolCall.id) {
        foundToolResult = true;
        continue;
      }
      if (foundToolResult && isUserMessage(msg)) {
        return msg;
      }
    }
    return null;
  }, [messages, maybeResult, toolCall.id]);

  const getContentText = useCallback((content: unknown): string => {
    if (typeof content === "string") return content;
    if (!Array.isArray(content)) return "";
    for (const item of content) {
      if (typeof item === "object" && item !== null) {
        const obj = item as Record<string, unknown>;
        if (obj.type === "text" && typeof obj.text === "string") {
          return obj.text;
        }
        if (obj.m_type === "text" && typeof obj.m_content === "string") {
          return obj.m_content;
        }
      }
    }
    return "";
  }, []);

  const answeredViaForm = useMemo(() => {
    if (!nextUserMessage) return false;
    const content = getContentText(nextUserMessage.content);
    return content.startsWith(marker);
  }, [nextUserMessage, marker, getContentText]);

  const parsedAnswers = useMemo(() => {
    if (!answeredViaForm || !nextUserMessage || !data) return null;
    const content = getContentText(nextUserMessage.content);
    return parseAnswersFromMessage(content, data.questions);
  }, [answeredViaForm, nextUserMessage, data, getContentText]);

  const isAnswered = nativeQuestionSet
    ? nativeQuestionSet.status === "answered"
    : !hasNativeQuestionCapability && answeredViaForm && !!parsedAnswers;

  const hasNextMessage = !!nextUserMessage;

  useEffect(() => {
    if (nativeQuestionSet?.status === "answered" || hasNextMessage) {
      clearAskQuestionsDraft(toolCall.id);
      return;
    }

    const hasAnswers = Object.keys(answers).length > 0;
    if (!hasAnswers && additionalText.trim().length === 0) {
      clearAskQuestionsDraft(toolCall.id);
      return;
    }

    saveAskQuestionsDraft(toolCall.id, answers, additionalText);
  }, [
    additionalText,
    answers,
    hasNextMessage,
    nativeQuestionSet?.status,
    toolCall.id,
  ]);

  const handleSubmit = useCallback(() => {
    if (!data) return;
    const formatted = formatAnswers(
      marker,
      data.questions,
      answers,
      additionalText,
    );
    void submit(formatted);
  }, [data, marker, answers, additionalText, submit]);

  const handleNativeSubmit = useCallback(async () => {
    if (
      !nativeQuestionSet ||
      nativeQuestionSet.status !== "pending" ||
      isSubmitting ||
      submissionSent
    ) {
      return;
    }
    const nativeAnswers: PendingQuestionAnswer[] =
      nativeQuestionSet.questions.map((question) => {
        const answer = answers[question.id];
        if (question.type === "yes_no") {
          return { id: question.id, answer: answer === "Yes" };
        }
        return {
          id: question.id,
          answer:
            typeof answer === "string" || Array.isArray(answer) ? answer : "",
        };
      });
    setIsSubmitting(true);
    setSubmitError(null);
    submissionInstance.current = questionInstance;
    try {
      const clientRequestId = await answerQuestions(
        threadId,
        nativeQuestionSet.tool_call_id,
        nativeAnswers,
        config,
        apiKey ?? undefined,
        nativeQuestionSet.asked_at_ms,
      );
      if (currentQuestionInstance.current !== questionInstance) return;
      setSubmissionRequestId(clientRequestId);
      setSubmissionSent(true);
    } catch (error) {
      if (currentQuestionInstance.current !== questionInstance) return;
      setSubmitError(
        error instanceof Error ? error.message : "Unable to submit answers",
      );
    } finally {
      if (currentQuestionInstance.current === questionInstance)
        setIsSubmitting(false);
    }
  }, [
    answers,
    apiKey,
    config,
    isSubmitting,
    nativeQuestionSet,
    questionInstance,
    submissionSent,
    threadId,
  ]);

  useEffect(() => {
    if (nativeQuestionSet?.status !== "pending") {
      setSubmissionSent(false);
      setSubmissionRequestId(null);
    }
  }, [nativeQuestionSet?.status]);

  useEffect(() => {
    if (
      !submissionAck ||
      submissionAck.accepted ||
      submissionInstance.current !== questionInstance
    )
      return;
    setSubmissionSent(false);
    setSubmissionRequestId(null);
    setSubmitError("The server rejected these answers. Please try again.");
    dispatch(requestSseRefresh({ chatId: threadId }));
  }, [dispatch, questionInstance, submissionAck, threadId]);

  useEffect(() => {
    if (!submissionCancelled || submissionInstance.current !== questionInstance)
      return;
    setSubmissionSent(false);
    setSubmissionRequestId(null);
    setSubmitError(null);
  }, [questionInstance, submissionCancelled]);

  const syncRefreshRequestedRef = useRef(false);
  useEffect(() => {
    if (nativeQuestionSet) {
      syncRefreshRequestedRef.current = false;
      return;
    }
    if (!hasNativeQuestionCapability || !data || hasNextMessage) return;
    if (syncRefreshRequestedRef.current) return;
    syncRefreshRequestedRef.current = true;
    dispatch(requestSseRefresh({ chatId: threadId }));
  }, [
    dispatch,
    hasNativeQuestionCapability,
    data,
    hasNextMessage,
    nativeQuestionSet,
    threadId,
  ]);

  if (nativeQuestionSet?.response_protocol_version === 2) {
    return (
      <NativeQuestionResponse
        set={nativeQuestionSet}
        threadId={threadId}
        toolCall={toolCall}
      />
    );
  }

  const questions = nativeQuestionSet?.questions ?? data?.questions ?? [];
  const instance = toolCall.id ?? "ask-questions";
  const widgets = (items: QuestionItem[], disabled: boolean) =>
    items.map((question) => (
      <QuestionWidget
        key={question.id}
        question={question}
        value={
          answers[question.id] || (question.type === "multi_select" ? [] : "")
        }
        onChange={(value) =>
          setAnswers((previous) => ({ ...previous, [question.id]: value }))
        }
        disabled={disabled}
      />
    ));

  if (nativeQuestionSet) {
    const isPending = nativeQuestionSet.status === "pending";
    const hasCompleteAnswers = nativeQuestionSet.questions.every((question) => {
      const answer = answers[question.id];
      if (question.type === "multi_select") return Array.isArray(answer);
      return typeof answer === "string" && answer.trim().length > 0;
    });
    const submittedAnswers = new Map(
      nativeQuestionSet.answers?.map((answer) => [answer.id, answer.answer]) ??
        [],
    );
    const state: QuestionState = isPending
      ? submitError
        ? "send_failed"
        : submissionSent
          ? "sent"
          : "pending"
      : nativeQuestionSet.status === "answered"
        ? "answered"
        : nativeQuestionSet.status === "cancelled"
          ? "cancelled"
          : "unknown";
    const entries: AnswerEntry[] = nativeQuestionSet.questions.map((q) => {
      const answer = submittedAnswers.get(q.id);
      return {
        id: q.id,
        question: q.text,
        answer: Array.isArray(answer)
          ? answer.join(", ")
          : answer === undefined
            ? "(no answer)"
            : typeof answer === "boolean"
              ? answer
                ? "Yes"
                : "No"
              : String(answer),
      };
    });
    return (
      <QuestionTranscript
        actions={
          isPending ? (
            <Flex direction="column" gap="2">
              {widgets(
                nativeQuestionSet.questions,
                isSubmitting || submissionSent,
              )}
              {submissionSent && (
                <Text color="gray" size="2">
                  Answers sent, awaiting confirmation.
                </Text>
              )}
              <Button
                onClick={() => void handleNativeSubmit()}
                disabled={isSubmitting || submissionSent || !hasCompleteAnswers}
              >
                {isSubmitting ? "Submitting…" : "Submit Answers"}
              </Button>
            </Flex>
          ) : null
        }
        error={submitError}
        input={
          isPending ? null : (
            <QuestionList questions={questions} instance={instance} />
          )
        }
        output={isPending ? null : <AnswerList answers={entries} />}
        questions={questions}
        state={state}
        toolCall={toolCall}
      />
    );
  }

  if (!hasNativeQuestionCapability && !hasNextMessage && data) {
    return (
      <QuestionTranscript
        actions={
          <Flex direction="column" gap="2">
            {widgets(data.questions, false)}
            <Box className={styles.questionItem}>
              <Text size="1" color="gray" mb="1" as="p">
                Additional comments (optional)
              </Text>
              <FieldTextarea
                value={additionalText}
                onChange={setAdditionalText}
                placeholder="Add any extra context..."
              />
            </Box>
            <Button onClick={handleSubmit}>Submit Answers</Button>
          </Flex>
        }
        questions={questions}
        state="pending"
        toolCall={toolCall}
      />
    );
  }

  if (isAnswered && data && parsedAnswers) {
    return (
      <QuestionTranscript
        input={<QuestionList questions={questions} instance={instance} />}
        output={
          <AnswerList
            answers={data.questions.map((q) => ({
              id: q.id,
              question: q.text,
              answer: parsedAnswers[q.id] || "(no answer)",
            }))}
            comments={parsedAnswers.__additional__}
          />
        }
        questions={questions}
        state="answered"
        toolCall={toolCall}
      />
    );
  }

  const fallbackUnrecorded = maybeResult ? undefined : unrecorded;
  const fallbackState: QuestionState = maybeResult?.tool_failed
    ? "failed"
    : hasNativeQuestionCapability && data && !hasNextMessage
      ? "syncing"
      : "unknown";
  return (
    <QuestionTranscript
      input={
        questions.length > 0 ? (
          <QuestionList questions={questions} instance={instance} />
        ) : null
      }
      output={
        fallbackState === "syncing" ? (
          <Text color="gray" size="2">
            Waiting for the current question state.
          </Text>
        ) : null
      }
      questions={questions}
      state={fallbackState}
      toolCall={toolCall}
      unrecorded={fallbackUnrecorded}
    />
  );
};

export default AskQuestionsTool;
