import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";

import { IconButton, LoadingState, Tooltip } from "../../components/ui";
import {
  reviewErrorMessage,
  useGetReviewStackByChatQuery,
  useGetReviewStackMemberQuery,
  useGetReviewStackCumulativeQuery,
  useGetReviewStackCumulativeFileQuery,
  useRefreshReviewStackMutation,
  type Review,
  type StackManifest,
  type ReviewStackCumulative,
} from "../../services/refact/review";
import { ReviewCanvas, StackHeaderContext } from "./ReviewCanvas";
import { ReviewHeader } from "./ReviewHeader";
import { ReviewReadOnlyContext } from "./readOnly";
import { ReviewStackNavigator } from "./ReviewStackNavigator";
import { DiffEditorsHost } from "../Workspace/GitPanel/diff/DiffEditorsHost";
import { newRequestKey, shortOid } from "./model";
import styles from "./ReviewStackSurface.module.css";

function CumulativeFile({
  chatId,
  memberId,
  comparison,
  path,
}: {
  chatId: string;
  memberId: string;
  comparison: ReviewStackCumulative;
  path: string;
}) {
  const { currentData, error, isFetching } =
    useGetReviewStackCumulativeFileQuery({
      chatId,
      memberId,
      path,
      stackRevision: comparison.stack_revision,
    });
  if (!currentData)
    return error ? (
      <p role="alert">{reviewErrorMessage(error)}</p>
    ) : (
      <LoadingState label="Loading stack comparison" />
    );
  if (isFetching) return <LoadingState label="Refreshing stack comparison" />;
  const { base, head } = currentData;
  if (base.kind === "binary" || head.kind === "binary")
    return <p>Binary content cannot be shown as a text diff.</p>;
  return (
    <DiffEditorsHost
      mode="side-by-side"
      left={base.kind === "text" ? base.text : ""}
      right={head.kind === "text" ? head.text : ""}
      languagePath={path}
      leftLabel={`Stack base ${shortOid(comparison.candidate.base_oid)}`}
      rightLabel={`Stack head ${shortOid(comparison.candidate.head_oid)}`}
      collapseUnchanged={false}
      onController={() => undefined}
    />
  );
}

function CumulativeView({
  chatId,
  memberId,
  stack,
}: {
  chatId: string;
  memberId: string;
  stack: StackManifest;
}) {
  const { currentData, error } = useGetReviewStackCumulativeQuery({
    chatId,
    memberId,
    stackRevision: stack.revision,
  });
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  if (error)
    return (
      <p className={styles.message} role="alert">
        {reviewErrorMessage(error)}
      </p>
    );
  if (!currentData) return <LoadingState label="Loading stack comparison" />;
  if (currentData.stack_revision !== stack.revision)
    return (
      <p className={styles.message} role="status">
        The stack changed. Refresh the comparison before inspecting it.
      </p>
    );
  const files = currentData.candidate.files.filter(
    (file) =>
      file.base?.digest !== file.head?.digest ||
      file.base_mode !== file.head_mode,
  );
  const path = files.some((file) => file.path === selectedPath)
    ? selectedPath
    : files.at(0)?.path ?? null;
  return (
    <section
      className={styles.comparison}
      aria-label="Cumulative stack comparison"
    >
      <p className={styles.notice}>
        Combined stack comparison · {shortOid(currentData.candidate.base_oid)} →{" "}
        {shortOid(currentData.candidate.head_oid)} · read-only. Discussions
        belong to each branch’s parent-relative review.
      </p>
      <div className={styles.files}>
        <nav aria-label="Stack comparison files" className={styles.fileList}>
          {files.map((file) => (
            <button
              type="button"
              className={styles.file}
              aria-current={file.path === path}
              key={file.path}
              title={file.path}
              onClick={() => setSelectedPath(file.path)}
            >
              {file.path.split("/").at(-1)}
            </button>
          ))}
        </nav>
        <div className={styles.diff}>
          {path ? (
            <CumulativeFile
              key={`${currentData.comparison_id}:${path}`}
              chatId={chatId}
              memberId={memberId}
              comparison={currentData}
              path={path}
            />
          ) : (
            <p className={styles.message}>
              No changes in this stack comparison.
            </p>
          )}
        </div>
      </div>
    </section>
  );
}

function MemberView({
  chatId,
  memberId,
  stale,
  active,
}: {
  chatId: string;
  memberId: string;
  stale: boolean;
  active: boolean;
}) {
  const { data, currentData, error } = useGetReviewStackMemberQuery({
    chatId,
    memberId,
  });
  const available = currentData ?? data;
  if (error && !available)
    return (
      <p className={styles.message} role="alert">
        {reviewErrorMessage(error)}
      </p>
    );
  if (available === undefined)
    return <LoadingState label="Loading branch review" />;
  if (available === null)
    return (
      <p className={styles.message}>
        This branch is in the review scope but has not been captured yet. Ask
        the owning chat to capture its pinned comparison.
      </p>
    );
  return (
    <>
      {error && (
        <p className={styles.notice} role="alert">
          Branch refresh failed. Showing its last loaded review read-only.
        </p>
      )}
      <ReviewReadOnlyContext.Provider value={stale || error !== undefined}>
        <ReviewCanvas key={available.id} review={available} active={active} />
      </ReviewReadOnlyContext.Provider>
    </>
  );
}

export function ReviewStackSurface({
  chatId,
  review,
  stale,
  ready,
}: {
  chatId: string;
  review: Review | null;
  stale: boolean;
  ready: boolean;
}) {
  const { currentData: stack, error: stackError } =
    useGetReviewStackByChatQuery({ chatId }, { skip: !ready });
  const [chosenMember, setChosenMember] = useState<string | null>(null);
  const [pendingNavigation, setPendingNavigation] = useState<{
    token: number;
    detail: unknown;
  }>();
  const [visitedMembers, setVisitedMembers] = useState<string[]>([]);
  const [comparison, setComparison] = useState<"parent" | "cumulative">(
    "parent",
  );
  const [refresh, refreshState] = useRefreshReviewStackMutation();
  useEffect(() => {
    const receive = (event: Event) => {
      if (!(event instanceof CustomEvent)) return;
      const detail: unknown = event.detail;
      if (
        typeof detail !== "object" ||
        detail === null ||
        !("reviewId" in detail) ||
        typeof detail.reviewId !== "string"
      )
        return;
      const member = stack?.members.find(
        (item) => item.review_id === detail.reviewId,
      );
      if (!member || member.state !== "ready") return;
      if (
        stack?.scope_choice.scope === "single" &&
        stack.scope_choice.selected_member_id !== member.id
      )
        return;
      setPendingNavigation({ token: Date.now(), detail });
      setVisitedMembers((current) =>
        Array.from(new Set([...current, member.id])),
      );
      setChosenMember(member.id);
      setComparison("parent");
    };
    window.addEventListener("refact:review-reveal", receive);
    return () => window.removeEventListener("refact:review-reveal", receive);
  }, [stack]);
  if (!stack)
    return review ? (
      <ReviewCanvas key={review.id} review={review} />
    ) : stackError ? (
      <p className={styles.message} role="alert">
        {reviewErrorMessage(stackError)}
      </p>
    ) : null;
  const active = stack.members.find(
    (member) => member.review_id === review?.id,
  );
  const recorded = stack.members.some(
    (member) => member.reason?.startsWith("entry:recorded_comparisons:"),
  );
  const effectiveComparison = recorded ? "parent" : comparison;
  const refreshLabel = recorded
    ? "Refresh is unavailable for immutable recorded comparison history"
    : "Refresh local stack refs";
  const memberId = stack.members.some((member) => member.id === chosenMember)
    ? chosenMember
    : active?.id ?? stack.scope_choice.selected_member_id;
  const stackControls = (
    <div className={styles.navigation}>
      <ReviewStackNavigator
        stack={stack}
        selectedMemberId={memberId}
        activeReviewId={review?.id}
        onSelect={(id) => {
          setVisitedMembers((ids) =>
            Array.from(new Set([...ids, memberId ?? id, id])),
          );
          setChosenMember(id);
          setComparison("parent");
        }}
        comparison={effectiveComparison}
        onComparisonChange={(value) =>
          setComparison(
            !recorded && stack.scope_choice.scope === "whole"
              ? value
              : "parent",
          )
        }
      />
      <Tooltip>
        <Tooltip.Trigger asChild>
          <IconButton
            icon={RefreshCw}
            aria-label="Refresh stack refs"
            size="sm"
            variant="ghost"
            disabled={recorded || stale || refreshState.isLoading}
            title={refreshLabel}
            loading={refreshState.isLoading}
            onClick={() =>
              !recorded &&
              void refresh({
                chatId,
                expected_revision: stack.revision,
                request_key: newRequestKey(),
              })
            }
          />
        </Tooltip.Trigger>
        <Tooltip.Content>{refreshLabel}</Tooltip.Content>
      </Tooltip>
    </div>
  );
  const status = (
    <span
      title={`Scope: ${stack.scope_choice.scope} · Browsing ${
        stack.members.find((member) => member.id === memberId)?.branch_ref ??
        "none"
      } · Execution: ${active?.branch_ref ?? "none"}`}
    >
      Scope: {stack.scope_choice.scope}
      {active?.id !== memberId && " · browsing only"}
    </span>
  );
  return (
    <StackHeaderContext.Provider
      value={{ controls: stackControls, status, pending: pendingNavigation }}
    >
      <div className={styles.surface}>
        {(effectiveComparison === "cumulative" ||
          !stack.members.find((member) => member.id === memberId)
            ?.review_id) && (
          <ReviewHeader
            stackControls={stackControls}
            identity="Stack review"
            status={status}
          />
        )}
        {refreshState.error && (
          <p className={styles.notice} role="alert">
            {reviewErrorMessage(refreshState.error)}
          </p>
        )}
        {memberId &&
          effectiveComparison === "cumulative" &&
          stack.scope_choice.scope === "whole" && (
            <CumulativeView
              key={memberId}
              chatId={chatId}
              memberId={memberId}
              stack={stack}
            />
          )}
        {Array.from(
          new Set([...visitedMembers, ...(memberId ? [memberId] : [])]),
        )
          .filter((id) => stack.members.some((member) => member.id === id))
          .map((id) => (
            <div
              key={id}
              className={styles.memberPane}
              hidden={id !== memberId || effectiveComparison === "cumulative"}
            >
              <MemberView
                chatId={chatId}
                memberId={id}
                stale={stale}
                active={id === memberId && effectiveComparison === "parent"}
              />
            </div>
          ))}
      </div>
    </StackHeaderContext.Provider>
  );
}
