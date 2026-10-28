import { useCallback, useEffect, useRef, useState } from "react";
import { v4 as uuidv4 } from "uuid";
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
import {
  AnswerList,
  QuestionList,
  QuestionTranscript,
  type AnswerEntry,
  type QuestionState,
} from "./QuestionTranscript";

export function NativeQuestionResponse({
  set,
  threadId,
  toolCall,
}: {
  set: PendingQuestionSet;
  threadId: string;
  toolCall: ToolCall;
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
      {...{ set, threadId, toolCall, instance }}
    />
  );
}

function ResponseInstance({
  set,
  threadId,
  toolCall,
  instance,
}: {
  set: PendingQuestionSet;
  threadId: string;
  toolCall: ToolCall;
  instance: string;
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
  const state: QuestionState = pending
    ? error
      ? "send_failed"
      : sent
        ? "sent"
        : "pending"
    : set.status === "answered"
      ? "answered"
      : set.status === "cancelled"
        ? "cancelled"
        : "unknown";
  const answers: AnswerEntry[] = set.questions.map((q) => {
    const answer = submitted?.find((a) => a.id === q.id);
    return {
      id: q.id,
      question: q.text,
      answer:
        set.status === "cancelled"
          ? "No response submitted"
          : answer
            ? `${
                answer.disposition === "custom" ? "Custom answer: " : ""
              }${responseLabel(answer)}`
            : "No answer",
      explanation:
        answer &&
        "explanation" in answer &&
        typeof answer.explanation === "string"
          ? answer.explanation
          : undefined,
    };
  });
  const status = (
    <>
      {sending && <p role="status">Submitting response…</p>}
      {sent && pending && (
        <div role="status" className={styles.sentStatus}>
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
    </>
  );
  return (
    <QuestionTranscript
      actions={
        pending ? (
          <div className={styles.responseForm}>
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
            {status}
          </div>
        ) : null
      }
      error={error}
      input={
        pending ? null : (
          <QuestionList questions={set.questions} instance={instance} />
        )
      }
      output={
        pending ? null : (
          <AnswerList
            answers={answers}
            comments={set.response?.overall_comments}
          />
        )
      }
      questions={set.questions}
      state={state}
      toolCall={toolCall}
    />
  );
}
