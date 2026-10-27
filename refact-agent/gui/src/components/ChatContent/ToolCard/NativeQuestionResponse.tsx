import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { v4 as uuidv4 } from "uuid";
import { CircleCheck, CircleHelp, CircleMinus } from "lucide-react";
import { ToolCard } from "./ToolCard";
import { Markdown } from "../../Markdown";
import { Button } from "../../ui";
import { QuestionResponseForm } from "./QuestionResponseForm";
import type {
  PendingQuestionSet,
  QuestionResponse,
  ToolCall,
} from "../../../services/refact/types";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { useAppDispatch } from "../../../hooks/useAppDispatch";
import {
  selectApiKey,
  selectConfig,
} from "../../../features/Config/configSlice";
import {
  selectCommandAckByThreadAndId,
  selectCommandCancelledByThreadAndId,
} from "../../../features/Chat/Thread/selectors";
import { requestSseRefresh } from "../../../features/Chat/Thread/actions";
import { submitQuestionResponse } from "../../../services/refact/chatCommands";
import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import {
  clearAskQuestionsDraft,
  loadAskQuestionsDraft,
  saveAskQuestionsDraft,
} from "../../../utils/chatUiPersistence";
import {
  responseLabel,
  type QuestionDraft,
} from "../../../utils/questionResponses";
import styles from "./AskQuestionsTool.module.css";
import { QuestionArtifactContext } from "../../Markdown/questionArtifactContext";
import { CompactToolDetails } from "../../Tools/AgenticTool/CompactToolDetails";
import { compactText } from "../../Tools/AgenticTool/compactPresentation";

function ResponseHistory({
  compact,
  count,
  children,
}: {
  compact: boolean;
  count: number;
  children: ReactNode;
}) {
  return compact ? (
    <CompactToolDetails facts={[{ label: "Questions", value: String(count) }]}>
      {children}
    </CompactToolDetails>
  ) : (
    <>{children}</>
  );
}

export function NativeQuestionResponse({
  set,
  threadId,
  toolCall,
  isOpen,
  onToggle,
  compact = false,
}: {
  set: PendingQuestionSet;
  threadId: string;
  toolCall: ToolCall;
  isOpen: boolean;
  onToggle: () => void;
  compact?: boolean;
}) {
  const config = useAppSelector(selectConfig);
  const identity = getEngineEndpointIdentity(config);
  const instance = JSON.stringify([
    identity,
    threadId,
    set.tool_call_id,
    set.asked_at_ms,
  ]);
  return (
    <ResponseInstance
      key={instance}
      {...{ set, threadId, toolCall, isOpen, onToggle, instance, compact }}
    />
  );
}

function ResponseInstance({
  set,
  threadId,
  toolCall,
  isOpen,
  onToggle,
  instance,
  compact,
}: {
  set: PendingQuestionSet;
  threadId: string;
  toolCall: ToolCall;
  isOpen: boolean;
  onToggle: () => void;
  instance: string;
  compact: boolean;
}) {
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const dispatch = useAppDispatch();
  const [drafts, setDrafts] = useState<Partial<Record<string, QuestionDraft>>>(
    () => loadAskQuestionsDraft(instance)?.responses ?? {},
  );
  const [comments, setComments] = useState(
    () => loadAskQuestionsDraft(instance)?.additionalText ?? "",
  );
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [requestId, setRequestId] = useState<string | null>(null);
  const lock = useRef(false);
  const retry = useRef<{ id: string; payload: string } | null>(null);
  const cancelledId = useRef<string | null>(null);
  const ack = useAppSelector((s) =>
    selectCommandAckByThreadAndId(s, threadId, requestId),
  );
  const cancelled = useAppSelector((s) =>
    selectCommandCancelledByThreadAndId(s, threadId, requestId),
  );
  const pending = set.status === "pending";

  useEffect(() => {
    if (!pending) {
      clearAskQuestionsDraft(instance);
      return;
    }
    saveAskQuestionsDraft(instance, {}, comments, drafts);
  }, [comments, drafts, instance, pending]);

  useEffect(() => {
    if (!ack || ack.accepted) return;
    lock.current = false;
    setSent(false);
    setError(
      "The server rejected this response. Your draft is kept; refresh and try again.",
    );
    retry.current = null;
    dispatch(requestSseRefresh({ chatId: threadId }));
  }, [ack, dispatch, threadId]);

  useEffect(() => {
    if (!cancelled) return;
    cancelledId.current = requestId;
    lock.current = false;
    retry.current = null;
    setSent(false);
    setError(null);
    setRequestId(null);
  }, [cancelled, requestId]);

  const submit = useCallback(
    async (response: QuestionResponse) => {
      if (lock.current || !pending) return;
      lock.current = true;
      setSending(true);
      setError(null);
      const payload = JSON.stringify(response);
      if (retry.current?.payload !== payload)
        retry.current = { id: uuidv4(), payload };
      const id = retry.current.id;
      setRequestId(id);
      try {
        await submitQuestionResponse(
          threadId,
          set.tool_call_id,
          set.asked_at_ms,
          response,
          id,
          config,
          apiKey ?? undefined,
        );
        if (cancelledId.current === id) return;
        setSent(true);
        dispatch(requestSseRefresh({ chatId: threadId }));
      } catch (e) {
        lock.current = false;
        if (cancelledId.current === id) return;
        setError(
          e instanceof Error
            ? e.message
            : "Unable to send response. Your draft is kept.",
        );
      } finally {
        setSending(false);
      }
    },
    [
      apiKey,
      config,
      dispatch,
      pending,
      set.asked_at_ms,
      set.tool_call_id,
      threadId,
    ],
  );

  const submitted =
    set.response?.answers ??
    set.answers?.map((answer) => ({
      ...answer,
      disposition: "answered" as const,
    }));
  const questionText = compactText(set.questions[0]?.text, 120);
  const stateLabel = pending
    ? error
      ? "Needs answer · Send failed"
      : sent
        ? "Needs answer · Awaiting confirmation"
        : "Needs answer"
    : set.status === "answered"
      ? "Answered"
      : set.status === "cancelled"
        ? "Withdrawn"
        : "State unknown";
  const compactProps = compact
    ? {
        density: "compact" as const,
        summary: questionText
          ? `${questionText}${
              set.questions.length > 1
                ? ` (+${set.questions.length - 1} questions)`
                : ""
            }`
          : "Question history",
        meta: stateLabel,
        status: error
          ? ("error" as const)
          : pending
            ? ("running" as const)
            : set.status === "answered"
              ? ("success" as const)
              : ("idle" as const),
      }
    : {};
  return (
    <ToolCard
      icon={
        pending ? (
          <CircleHelp />
        ) : set.status === "cancelled" ? (
          <CircleMinus />
        ) : compact && set.status !== "answered" ? (
          <CircleHelp />
        ) : (
          <CircleCheck />
        )
      }
      summary={
        pending
          ? "Questions for you"
          : set.status === "cancelled"
            ? "Questions cancelled"
            : "Response submitted"
      }
      status={pending ? "running" : "success"}
      isOpen={isOpen}
      onToggle={onToggle}
      collapsible={compact || !pending}
      toolCall={toolCall}
      {...compactProps}
    >
      <div className={styles.responseForm}>
        {pending ? (
          <QuestionResponseForm
            questions={set.questions}
            instance={instance}
            drafts={drafts}
            comments={comments}
            onDrafts={setDrafts}
            onComments={setComments}
            disabled={sending || sent}
            approval={set.tool_call_id.startsWith("agent-approval:")}
            onSubmit={(r) => void submit(r)}
          />
        ) : (
          <ResponseHistory compact={compact} count={set.questions.length}>
            {set.questions.map((q) => {
              const answer = submitted?.find((a) => a.id === q.id);
              return (
                <section key={q.id} className={styles.responseQuestion}>
                  <QuestionArtifactContext.Provider
                    value={{
                      instance: `${instance}:${q.id}`,
                      enabled: false,
                      onAnswer: () => undefined,
                    }}
                  >
                    <Markdown>{q.text}</Markdown>
                  </QuestionArtifactContext.Provider>
                  {compact && q.options?.length ? (
                    <ul>
                      {q.options.map((option) => (
                        <li key={option}>{option}</li>
                      ))}
                    </ul>
                  ) : null}
                  <p className={styles.answerText}>
                    {set.status === "cancelled"
                      ? "No response submitted"
                      : answer
                        ? `${
                            answer.disposition === "custom"
                              ? "Custom answer: "
                              : ""
                          }${responseLabel(answer)}`
                        : "No answer"}
                  </p>
                  {answer &&
                    "explanation" in answer &&
                    typeof answer.explanation === "string" && (
                      <p className={styles.answerText}>{answer.explanation}</p>
                    )}
                </section>
              );
            })}
            {set.response?.overall_comments && (
              <div>
                <strong>Overall comments</strong>
                <p className={styles.answerText}>
                  {set.response.overall_comments}
                </p>
              </div>
            )}
          </ResponseHistory>
        )}
        {error && <p role="alert">{error}</p>}
        {sending && <p role="status">Submitting response…</p>}
        {sent && pending && (
          <div role="status">
            <p>Response sent, awaiting confirmation.</p>
            <Button
              size="sm"
              onClick={() => {
                lock.current = false;
                setSent(false);
                dispatch(requestSseRefresh({ chatId: threadId }));
              }}
            >
              Check status / retry
            </Button>
          </div>
        )}
      </div>
    </ToolCard>
  );
}
