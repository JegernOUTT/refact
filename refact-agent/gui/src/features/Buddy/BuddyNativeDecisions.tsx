import { useEffect, useMemo, useRef, useState } from "react";
import { CircleHelp, RefreshCw } from "lucide-react";
import { Button, IconButton, Text } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import { v4 as uuidv4 } from "uuid";
import { buddyOperatorIdentity } from "../../services/refact/buddyOperator";
import {
  useGetBuddyNativeDecisionsQuery,
  useGetBuddyNativeQuestionQuery,
  useAnswerBuddyNativeQuestionMutation,
  type NativeQuestionView,
} from "../../services/refact/buddyDecisions";
import { useGetBuddyNativeWorkQuery } from "../../services/refact/buddyNativeWork";
import type {
  PendingQuestionAnswer,
  QuestionResponse,
} from "../../services/refact/types";
import { QuestionResponseForm } from "../../components/ChatContent/ToolCard/QuestionResponseForm";
import type { QuestionDraft } from "../../utils/questionResponses";
import {
  clearAskQuestionsDraft,
  loadAskQuestionsDraft,
  saveAskQuestionsDraft,
} from "../../utils/chatUiPersistence";
import {
  BUDDY_TERMS,
  BuddyDisclosure,
  BuddyErrorCallout,
  BuddySection,
  humanizeBuddyIdentifier,
} from "./ui";
import styles from "./BuddyNativeDecisions.module.css";

const DECISION_POLLING_INTERVAL_MS = 5000;
const RETAINED_QUESTION_LIMIT = 20;
const RENDERED_QUESTION_LIMIT = 100;

function chatTitleKey(projectRoot: string, chatId: string): string {
  return JSON.stringify([projectRoot, chatId]);
}

function questionKey(row: NativeQuestionView): string {
  return JSON.stringify([
    row.project_root,
    row.chat_id,
    row.question.tool_call_id,
    row.question.asked_at_ms,
  ]);
}

function mergeNativeQuestions(
  previous: NativeQuestionView[],
  current: NativeQuestionView[],
): NativeQuestionView[] {
  const listed = new Set(current.map(questionKey));
  const carried = previous
    .filter((row) => !listed.has(questionKey(row)))
    .slice(0, RETAINED_QUESTION_LIMIT);
  const merged = [...current, ...carried].slice(0, RENDERED_QUESTION_LIMIT);
  const unchanged =
    merged.length === previous.length &&
    merged.every(
      (row, index) => JSON.stringify(row) === JSON.stringify(previous[index]),
    );
  return unchanged ? previous : merged;
}

function QuestionForm({
  initial,
  identity,
  chatTitle,
}: {
  initial: NativeQuestionView;
  identity: string;
  chatTitle?: string;
}) {
  const [answerQuestion] = useAnswerBuddyNativeQuestionMutation();
  const draftKey = JSON.stringify([identity, questionKey(initial)]);
  const [answers, setAnswers] = useState<Record<string, string | string[]>>({});
  const [drafts, setDrafts] = useState<Partial<Record<string, QuestionDraft>>>(
    () => loadAskQuestionsDraft(draftKey)?.responses ?? {},
  );
  const [comments, setComments] = useState(
    () => loadAskQuestionsDraft(draftKey)?.additionalText ?? "",
  );
  const retry = useRef<{ id: string; payload: string } | null>(null);
  const [sent, setSent] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState(false);
  const [rechecking, setRechecking] = useState(false);
  const lock = useRef(false);
  const refetchStatusRef = useRef<() => void>(() => undefined);
  const visible = useIsDocumentVisible(() => refetchStatusRef.current());
  const status = useGetBuddyNativeQuestionQuery(
    {
      project_root: initial.project_root,
      chat_id: initial.chat_id,
      tool_call_id: initial.question.tool_call_id,
      endpoint_identity: identity,
    },
    {
      pollingInterval: visible ? 3000 : 0,
      skipPollingIfUnfocused: true,
      refetchOnMountOrArgChange: true,
      refetchOnFocus: true,
      refetchOnReconnect: true,
    },
  );
  const question = status.currentData?.question;
  refetchStatusRef.current = () => {
    if (!status.error) void status.refetch();
  };
  const pending =
    question?.status === "pending" &&
    question.asked_at_ms === initial.question.asked_at_ms;
  const sameQuestions =
    JSON.stringify(question?.questions) ===
    JSON.stringify(initial.question.questions);
  const complete = initial.question.questions.every((q) => {
    const answer = answers[q.id];
    return (
      q.type === "multi_select" ||
      (typeof answer === "string" && answer.trim().length > 0)
    );
  });
  const fieldsDisabled =
    !pending || !sameQuestions || sent || sending || Boolean(status.error);
  useEffect(() => {
    if (initial.question.response_protocol_version !== 2) return;
    if (question && question.status !== "pending")
      clearAskQuestionsDraft(draftKey);
    else saveAskQuestionsDraft(draftKey, {}, comments, drafts);
  }, [
    comments,
    draftKey,
    drafts,
    initial.question.response_protocol_version,
    question,
  ]);
  const submit = async () => {
    if (
      lock.current ||
      !pending ||
      !sameQuestions ||
      !complete ||
      sent ||
      Boolean(status.error) ||
      status.isFetching
    )
      return;
    lock.current = true;
    setSending(true);
    setError(false);
    const payload: PendingQuestionAnswer[] = initial.question.questions.map(
      (q) => ({
        id: q.id,
        answer:
          q.type === "yes_no"
            ? answers[q.id] === "Yes"
            : q.type === "multi_select"
              ? Array.isArray(answers[q.id])
                ? answers[q.id]
                : []
              : answers[q.id],
      }),
    );
    try {
      await answerQuestion({
        chat_id: initial.chat_id,
        tool_call_id: initial.question.tool_call_id,
        project_root: initial.project_root,
        asked_at_ms: initial.question.asked_at_ms,
        endpoint_identity: identity,
        client_request_id: uuidv4(),
        answers: payload,
      }).unwrap();
      setSent(true);
      void status.refetch();
    } catch {
      setError(true);
    } finally {
      lock.current = false;
      setSending(false);
    }
  };
  const submitResponse = async (response: QuestionResponse) => {
    if (lock.current || fieldsDisabled || status.isFetching) return;
    lock.current = true;
    setSending(true);
    setError(false);
    const payload = JSON.stringify(response);
    if (retry.current?.payload !== payload)
      retry.current = { id: uuidv4(), payload };
    try {
      await answerQuestion({
        chat_id: initial.chat_id,
        tool_call_id: initial.question.tool_call_id,
        project_root: initial.project_root,
        asked_at_ms: initial.question.asked_at_ms,
        endpoint_identity: identity,
        client_request_id: retry.current.id,
        response,
      }).unwrap();
      setSent(true);
      void status.refetch();
    } catch {
      setError(true);
    } finally {
      lock.current = false;
      setSending(false);
    }
  };
  const chatLabel = humanizeBuddyIdentifier(initial.chat_id);
  return (
    <li className={styles.question} aria-label="Question">
      <Text as="h3" size="2" weight="medium" className={styles.questionTitle}>
        {chatTitle ?? "Question from an untitled chat"}
      </Text>
      {!chatTitle && (
        <Text
          as="p"
          size="1"
          className={styles.questionMeta}
          title={chatLabel.full}
        >
          {`Chat ${chatLabel.label}`}
        </Text>
      )}
      {initial.question.response_protocol_version === 2 ? (
        <QuestionResponseForm
          questions={initial.question.questions}
          instance={questionKey(initial)}
          drafts={drafts}
          comments={comments}
          onDrafts={setDrafts}
          onComments={setComments}
          disabled={fieldsDisabled || status.isFetching}
          approval={initial.question.tool_call_id.startsWith("agent-approval:")}
          onSubmit={(response) => void submitResponse(response)}
        />
      ) : (
        initial.question.questions.map((q) => {
          const group = `${initial.chat_id}:${initial.question.tool_call_id}:${q.id}`;
          return (
            <div
              key={q.id}
              role="group"
              aria-labelledby={`${group}:label`}
              className={styles.answerGroup}
              data-disabled={fieldsDisabled ? "true" : undefined}
            >
              <Text
                as="p"
                size="1"
                id={`${group}:label`}
                className={styles.answerLabel}
              >
                {q.text}
              </Text>
              {q.type === "free_text" ? (
                <textarea
                  className={styles.answerInput}
                  aria-label={q.text}
                  disabled={fieldsDisabled}
                  value={typeof answers[q.id] === "string" ? answers[q.id] : ""}
                  onChange={(event) =>
                    setAnswers((a) => ({ ...a, [q.id]: event.target.value }))
                  }
                />
              ) : (
                (q.type === "yes_no" ? ["Yes", "No"] : q.options ?? []).map(
                  (option) => (
                    <label key={option} className={styles.choice}>
                      <input
                        type={q.type === "multi_select" ? "checkbox" : "radio"}
                        name={group}
                        disabled={fieldsDisabled}
                        checked={
                          q.type === "multi_select"
                            ? Array.isArray(answers[q.id]) &&
                              answers[q.id].includes(option)
                            : answers[q.id] === option
                        }
                        onChange={(event) =>
                          setAnswers((a) => {
                            const value = a[q.id];
                            const selected = Array.isArray(value) ? value : [];
                            return {
                              ...a,
                              [q.id]:
                                q.type !== "multi_select"
                                  ? option
                                  : event.target.checked
                                    ? [...selected, option]
                                    : selected.filter(
                                        (value) => value !== option,
                                      ),
                            };
                          })
                        }
                      />
                      {option}
                    </label>
                  ),
                )
              )}
            </div>
          );
        })
      )}
      <div className={styles.actions}>
        {initial.question.response_protocol_version !== 2 && (
          <Button
            size="sm"
            disabled={
              !pending ||
              !sameQuestions ||
              !complete ||
              sent ||
              sending ||
              Boolean(status.error) ||
              status.isFetching
            }
            onClick={() => void submit()}
          >
            {sending ? "Submitting answer…" : "Submit answer"}
          </Button>
        )}
        {sent && pending && (
          <Button
            size="sm"
            variant="ghost"
            disabled={rechecking}
            onClick={() => {
              setRechecking(true);
              void status
                .refetch()
                .unwrap()
                .then((response) => {
                  if (
                    response.question.status === "pending" &&
                    response.question.asked_at_ms ===
                      initial.question.asked_at_ms
                  ) {
                    setSent(false);
                    setError(false);
                  }
                })
                .catch(() => setError(true))
                .finally(() => setRechecking(false));
            }}
          >
            Recheck before answering again
          </Button>
        )}
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            void status.refetch();
          }}
        >
          Refresh decision status
        </Button>
      </div>
      {sent && pending && (
        <Text as="p" size="1" role="status" className={styles.note}>
          Answer queued. Waiting for the chat to confirm; this is not yet
          resolved.
        </Text>
      )}
      {!status.error &&
        question?.status === "answered" &&
        question.asked_at_ms === initial.question.asked_at_ms &&
        sameQuestions && (
          <Text as="p" size="1" role="status" className={styles.note}>
            Answered — confirmed by the chat.
          </Text>
        )}
      {!status.error &&
        question?.status === "cancelled" &&
        question.asked_at_ms === initial.question.asked_at_ms &&
        sameQuestions && (
          <Text as="p" size="1" role="status" className={styles.note}>
            Cancelled in the chat. No answer will be applied.
          </Text>
        )}
      {question &&
        (!sameQuestions ||
          question.asked_at_ms !== initial.question.asked_at_ms) && (
          <Text as="p" size="1" role="alert" className={styles.alert}>
            This question changed. Refresh the question list before answering.
          </Text>
        )}
      {(error || status.error) && (
        <Text as="p" size="1" role="alert" className={styles.alert}>
          The chat hasn&apos;t confirmed this answer. Refresh status before
          retrying.
        </Text>
      )}
    </li>
  );
}

function Decisions({
  identity,
  hideError,
}: {
  identity: string;
  hideError: boolean;
}) {
  const [halted, setHalted] = useState(false);
  const refetchRef = useRef<() => void>(() => undefined);
  const visible = useIsDocumentVisible(() => refetchRef.current());
  const query = useGetBuddyNativeDecisionsQuery(
    { endpoint_identity: identity },
    {
      pollingInterval: visible && !halted ? DECISION_POLLING_INTERVAL_MS : 0,
      skipPollingIfUnfocused: true,
      refetchOnMountOrArgChange: true,
      refetchOnFocus: true,
      refetchOnReconnect: true,
    },
  );
  refetchRef.current = () => {
    if (!halted && !query.error) void query.refetch();
  };
  const work = useGetBuddyNativeWorkQuery(
    { endpoint_identity: identity },
    {
      refetchOnMountOrArgChange: true,
      refetchOnFocus: true,
      refetchOnReconnect: true,
    },
  );
  const nativeWork = work.currentData;
  const chatTitles = useMemo(() => {
    const titles = new Map<string, string>();
    if (!nativeWork) return titles;
    for (const session of nativeWork.sessions) {
      const label = session.title.trim();
      if (label.length > 0)
        titles.set(
          chatTitleKey(nativeWork.project_root, session.chat_id),
          label,
        );
    }
    return titles;
  }, [nativeWork]);
  const refetch = query.refetch;
  const [rendered, setRendered] = useState<NativeQuestionView[]>([]);
  const snapshot = query.currentData;
  const failed = Boolean(query.error);
  useEffect(() => {
    setHalted(failed);
  }, [failed]);
  useEffect(() => {
    if (!snapshot) return;
    setRendered((previous) =>
      mergeNativeQuestions(
        previous.filter((row) => row.project_root === snapshot.project_root),
        snapshot.questions,
      ),
    );
  }, [snapshot]);
  const questions = rendered.filter(
    (question) => !snapshot || question.project_root === snapshot.project_root,
  );
  const truncated = !failed && Boolean(snapshot?.truncated);
  if (!failed && !truncated && questions.length === 0) return null;
  const visibleQuestions = failed ? [] : questions;
  const first = visibleQuestions.at(0);
  const rest = visibleQuestions.slice(1);
  const renderQuestion = (question: NativeQuestionView) => (
    <QuestionForm
      key={`${questionKey(question)}:${JSON.stringify(
        question.question.questions,
      )}`}
      initial={question}
      identity={identity}
      chatTitle={chatTitles.get(
        chatTitleKey(question.project_root, question.chat_id),
      )}
    />
  );
  if (failed && hideError) return null;
  return (
    <BuddySection
      title={BUDDY_TERMS.questions}
      icon={CircleHelp}
      count={visibleQuestions.length > 0 ? visibleQuestions.length : undefined}
      testId="buddy-home-questions"
      action={
        <IconButton
          size="sm"
          variant="ghost"
          icon={RefreshCw}
          aria-label="Refresh questions"
          onClick={() => void refetch()}
        />
      }
    >
      {failed && (
        <BuddyErrorCallout
          compact
          thing="questions"
          error={query.error}
          retrying={query.isFetching}
          onRetry={() => void refetch()}
        />
      )}
      {truncated && (
        <Text as="p" size="1" className={styles.note}>
          Some chats weren&apos;t checked. Open the remaining chats to see their
          questions.
        </Text>
      )}
      {first && (
        <ul className={styles.questionList}>{renderQuestion(first)}</ul>
      )}
      {rest.length > 0 && (
        <BuddyDisclosure
          label={`${rest.length} more question${rest.length === 1 ? "" : "s"}`}
          persistKey="buddy.home.moreQuestions"
        >
          <ul className={styles.questionList}>{rest.map(renderQuestion)}</ul>
        </BuddyDisclosure>
      )}
    </BuddySection>
  );
}

export function BuddyNativeDecisions({
  hideError = false,
}: {
  hideError?: boolean;
}) {
  const identity = useAppSelector(buddyOperatorIdentity);
  return <Decisions key={identity} identity={identity} hideError={hideError} />;
}
