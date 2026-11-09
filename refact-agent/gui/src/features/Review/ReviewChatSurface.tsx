import {
  useEffect,
  useCallback,
  useId,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";

import { Button, SegmentedControl } from "../../components/ui";
import {
  ArtifactPolicyProvider,
  type ArtifactPolicy,
} from "../../components/Markdown/ArtifactPolicyContext";
import {
  reviewApi,
  reviewErrorMessage,
  useGetReviewByChatQuery,
  useGetReviewStackByChatQuery,
} from "../../services/refact/review";
import { useAppSelector } from "../../hooks";
import {
  selectRuntimeById,
  selectThreadModeById,
} from "../Chat/Thread/selectors";
import {
  ReviewSurfaceActiveContext,
  ReviewAcceptedNavigationContext,
} from "./ReviewCanvas";
import { ReviewStackSurface } from "./ReviewStackSurface";
import { ReviewReadOnlyContext } from "./readOnly";
import styles from "./Review.module.css";
import { ReviewSplitHandle } from "./ReviewSplitHandle";
import splitStyles from "./ReviewSplitHandle.module.css";

export type ReviewChatSurfaceProps = {
  chatId: string;
  children: ReactNode;
};

const REVIEW_POLL_MS = 15000;
const REVIEW_ARTIFACT_POLICY: ArtifactPolicy = {
  mode: "review-seamless",
};

/**
 * The chat stays in the same tree position whether or not a review exists,
 * so a review appearing, refreshing or failing never remounts it.
 */
export function ReviewChatSurface({
  chatId,
  children,
}: ReviewChatSurfaceProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chatSlotId = useId();
  const canvasSlotId = useId();
  const [focusedSurface, setFocusedSurface] = useState("chat");
  const [containerWidth, setContainerWidth] = useState(0);
  const [chatWidth, setChatWidth] = useState(320);
  useEffect(() => {
    const element = containerRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      setContainerWidth(entries[0].contentRect.width);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const ready = useAppSelector(
    (state) => selectRuntimeById(state, chatId)?.snapshot_received ?? false,
  );
  const cached = useAppSelector(
    (state) =>
      reviewApi.endpoints.getReviewByChat.select({ chatId })(state).data,
  );
  const { data, currentData, error, isFetching, refetch } =
    useGetReviewByChatQuery(
      { chatId },
      {
        skip: !ready,
        refetchOnMountOrArgChange: true,
        refetchOnFocus: true,
        refetchOnReconnect: true,
        skipPollingIfUnfocused: true,
        pollingInterval: REVIEW_POLL_MS,
      },
    );
  const mode = useAppSelector((state) => selectThreadModeById(state, chatId));
  const stackQuery = useGetReviewStackByChatQuery(
    { chatId },
    {
      skip: !ready,
      refetchOnMountOrArgChange: true,
      refetchOnFocus: true,
      refetchOnReconnect: true,
      skipPollingIfUnfocused: true,
      pollingInterval: REVIEW_POLL_MS,
    },
  );
  const stack =
    stackQuery.data?.owner_chat_id === chatId ? stackQuery.data : null;
  const available = ready ? data : cached;
  const review = available?.owner_chat_id === chatId ? available : null;
  const stale =
    review !== null &&
    (!ready || error !== undefined || currentData === undefined);
  const unavailable = ready && review !== null && error !== undefined;
  const hasWorkspace = review !== null || stack !== null;
  const narrow = containerWidth > 0 && containerWidth <= 680;
  const resizable = hasWorkspace && !narrow && containerWidth > 0;
  const acceptNavigation = useCallback(
    (reviewId: string) => {
      if (
        reviewId !== review?.id &&
        !stack?.members.some(
          (member) =>
            member.review_id === reviewId &&
            member.state === "ready" &&
            (stack.scope_choice.scope !== "single" ||
              stack.scope_choice.selected_member_id === member.id),
        )
      )
        return;
      setFocusedSurface("canvas");
    },
    [review?.id, stack],
  );
  const maxWidth = Math.max(272, containerWidth - 400);
  const width = Math.min(maxWidth, Math.max(272, chatWidth));
  const splitStyle = { "--review-chat-width": `${width}px` } as CSSProperties;

  return (
    <div className={styles.surfaceContainer} ref={containerRef}>
      {hasWorkspace && narrow && (
        <div className={styles.focusSwitch}>
          <SegmentedControl
            aria-label="Review surface"
            aria-controls={`${chatSlotId} ${canvasSlotId}`}
            value={focusedSurface}
            onValueChange={setFocusedSurface}
            options={[
              { value: "chat", label: "Chat" },
              {
                value: "canvas",
                label: "Canvas",
              },
            ]}
          />
        </div>
      )}
      <div
        className={`${styles.surface} ${
          resizable ? splitStyles.resizable : ""
        }`}
        data-review={hasWorkspace}
        data-narrow={narrow}
        style={resizable ? splitStyle : undefined}
      >
        <div
          className={styles.chatSlot}
          id={chatSlotId}
          hidden={hasWorkspace && narrow && focusedSurface !== "chat"}
        >
          {!hasWorkspace && mode === "review" && (
            <p className={styles.orientation}>
              Review mode: describe what to review (a branch, commit range or
              your current changes) and the review canvas opens here once a
              candidate is captured.
            </p>
          )}
          {children}
        </div>
        {resizable && (
          <ReviewSplitHandle
            value={width}
            min={272}
            max={maxWidth}
            onChange={setChatWidth}
            controls={chatSlotId}
          />
        )}
        {hasWorkspace && (
          <div
            className={styles.canvasSlot}
            id={canvasSlotId}
            hidden={narrow && focusedSurface !== "canvas"}
          >
            {unavailable && (
              <div className={styles.notice} role="alert">
                <span className={styles.grow}>
                  Review unavailable: {reviewErrorMessage(error)}. Showing the
                  last loaded state read-only.
                </span>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  loading={isFetching}
                  onClick={() => void refetch()}
                >
                  Retry
                </Button>
              </div>
            )}
            <ReviewReadOnlyContext.Provider value={stale}>
              <ArtifactPolicyProvider value={REVIEW_ARTIFACT_POLICY}>
                <ReviewAcceptedNavigationContext.Provider
                  value={acceptNavigation}
                >
                  <ReviewSurfaceActiveContext.Provider
                    value={!narrow || focusedSurface === "canvas"}
                  >
                    <ReviewStackSurface
                      chatId={chatId}
                      review={review}
                      stale={
                        stale ||
                        !ready ||
                        (stack !== null && stackQuery.error !== undefined)
                      }
                      ready={ready}
                    />
                  </ReviewSurfaceActiveContext.Provider>
                </ReviewAcceptedNavigationContext.Provider>
              </ArtifactPolicyProvider>
            </ReviewReadOnlyContext.Provider>
          </div>
        )}
      </div>
    </div>
  );
}
