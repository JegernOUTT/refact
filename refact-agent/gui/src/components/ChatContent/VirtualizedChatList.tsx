/* eslint-disable react/prop-types */
import React, {
  useCallback,
  useRef,
  useState,
  useMemo,
  useLayoutEffect,
} from "react";
import { Virtuoso, VirtuosoHandle } from "react-virtuoso";
import { Flex, Container, Box } from "@radix-ui/themes";
import classNames from "classnames";
import { ScrollToBottomButton } from "../ScrollArea/ScrollToBottomButton";
import styles from "./ChatContent.module.css";

const SCROLL_INTENT_MS = 500;
const PASSIVE_SCROLL_GRACE_MS = 250;
const MIN_MEASURED_LIST_HEIGHT = 1;
const DEFAULT_ITEM_HEIGHT = 240;
const VIRTUOSO_MIN_OVERSCAN_ITEM_COUNT = { top: 20, bottom: 20 };
const VIRTUOSO_OVERSCAN = { main: 400, reverse: 400 };

function canScrollInWheelDirection(
  element: HTMLElement,
  deltaY: number,
): boolean {
  if (deltaY < 0) return element.scrollTop > 0;
  if (deltaY > 0) {
    return element.scrollTop + element.clientHeight < element.scrollHeight - 1;
  }
  return false;
}

function isWheelHandledByNestedScroller(
  scroller: HTMLElement,
  target: EventTarget | null,
  deltaY: number,
): boolean {
  if (!(target instanceof HTMLElement)) return false;

  let current: HTMLElement | null = target;
  while (current && current !== scroller) {
    const style = window.getComputedStyle(current);
    const canScrollY =
      style.overflowY === "auto" ||
      style.overflowY === "scroll" ||
      style.overflowY === "overlay";
    if (canScrollY && canScrollInWheelDirection(current, deltaY)) {
      return true;
    }
    current = current.parentElement;
  }

  return false;
}

export type VirtualizedChatListProps<T extends { key: string }> = {
  items: T[];
  renderItem: (item: T) => React.ReactNode;
  initialScrollIndex?: number;
  footer?: React.ReactNode;
  isStreaming?: boolean;
};

export function VirtualizedChatList<T extends { key: string }>({
  items,
  renderItem,
  initialScrollIndex,
  footer,
  isStreaming = false,
}: VirtualizedChatListProps<T>) {
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const [showFollowButton, setShowFollowButton] = useState(false);
  const autoFollowRef = useRef(true);
  const userScrolledUpRef = useRef(false);
  const lastScrollTopRef = useRef(0);
  const lastItemsSignatureRef = useRef<string | null>(null);
  const lastUserInputTsRef = useRef(0);
  const pointerDownRef = useRef(false);
  const suppressPassiveScrollUntilRef = useRef(0);
  const recentlyChangedOutputUntilRef = useRef(0);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const [hasMeasuredHeight, setHasMeasuredHeight] = useState(false);
  // Timestamp of the last active user input that should scroll downward.
  // Used to distinguish real user scroll-down from Virtuoso measurement
  // adjustments that passively change scrollTop.
  const lastActiveScrollDownTsRef = useRef(0);

  const markUserInput = useCallback(() => {
    lastUserInputTsRef.current = performance.now();
  }, []);

  useLayoutEffect(() => {
    const wrapper = wrapperRef.current;
    if (!wrapper) return;

    const updateMeasuredHeight = () => {
      setHasMeasuredHeight(
        wrapper.getBoundingClientRect().height >= MIN_MEASURED_LIST_HEIGHT,
      );
    };

    updateMeasuredHeight();
    const resizeObserver = new ResizeObserver(updateMeasuredHeight);
    resizeObserver.observe(wrapper);
    return () => resizeObserver.disconnect();
  }, []);

  const lastItemKey = items.length > 0 ? items[items.length - 1].key : "";
  const itemsSignature = `${items.length}:${lastItemKey}`;
  if (lastItemsSignatureRef.current !== itemsSignature) {
    lastItemsSignatureRef.current = itemsSignature;
    recentlyChangedOutputUntilRef.current =
      performance.now() + PASSIVE_SCROLL_GRACE_MS;
  }

  const handleAtBottomChange = useCallback((bottom: boolean) => {
    if (bottom && userScrolledUpRef.current) {
      // Only re-arm auto-follow if the user recently performed an active
      // scroll-down gesture (wheel or touch). Virtuoso measurement
      // adjustments can passively shift the scroll position into the
      // atBottomThreshold — that must NOT re-arm follow.
      const recentActiveScroll =
        performance.now() - lastActiveScrollDownTsRef.current <
        SCROLL_INTENT_MS;
      if (recentActiveScroll) {
        autoFollowRef.current = true;
        userScrolledUpRef.current = false;
      }
      // When NOT an active scroll we leave userScrolledUpRef = true so the
      // follow button reappears if Virtuoso later pushes us away from bottom.
    }
    setShowFollowButton(!bottom && userScrolledUpRef.current);
  }, []);

  const handleFollowClick = useCallback(() => {
    autoFollowRef.current = true;
    userScrolledUpRef.current = false;
    setShowFollowButton(false);
    if (items.length === 0) return;
    virtuosoRef.current?.scrollToIndex({
      index: items.length - 1,
      align: "end",
      behavior: "smooth",
    });
  }, [items.length]);

  const followOutput = useCallback(
    (isAtBottom: boolean) => {
      if (userScrolledUpRef.current) return false;
      if (
        !isStreaming &&
        performance.now() > recentlyChangedOutputUntilRef.current
      ) {
        return false;
      }
      if (isAtBottom || autoFollowRef.current) {
        suppressPassiveScrollUntilRef.current =
          performance.now() + PASSIVE_SCROLL_GRACE_MS;
        return "auto";
      }
      return false;
    },
    [isStreaming],
  );

  const computeItemKey = useCallback((_index: number, item: T) => item.key, []);

  const itemContent = useCallback(
    (_index: number, item: T) => (
      <Container
        className={styles.virtuosoItem}
        data-testid="chat-virtuoso-item"
      >
        {renderItem(item)}
      </Container>
    ),
    [renderItem],
  );

  const Scroller = useMemo(() => {
    const ScrollerComponent = React.forwardRef<
      HTMLDivElement,
      React.HTMLAttributes<HTMLDivElement>
    >(function VirtuosoScroller(props, ref) {
      const {
        children,
        style,
        className,
        onWheel,
        onScroll,
        onKeyDown,
        ...restProps
      } = props;
      const handleWheel: React.WheelEventHandler<HTMLDivElement> = (event) => {
        const wheelHandledByNestedScroller = isWheelHandledByNestedScroller(
          event.currentTarget,
          event.target,
          event.deltaY,
        );

        if (!wheelHandledByNestedScroller) {
          markUserInput();
          if (event.deltaY < 0) {
            autoFollowRef.current = false;
            userScrolledUpRef.current = true;
            setShowFollowButton(true);
          } else if (event.deltaY > 0) {
            lastActiveScrollDownTsRef.current = performance.now();
          }
        }

        onWheel?.(event);
      };

      const handleKeyDown: React.KeyboardEventHandler<HTMLDivElement> = (
        event,
      ) => {
        const scrollsDown =
          event.key === "End" ||
          event.key === "PageDown" ||
          event.key === "ArrowDown" ||
          (event.key === " " && !event.shiftKey);
        const scrollsUp =
          event.key === "Home" ||
          event.key === "PageUp" ||
          event.key === "ArrowUp" ||
          (event.key === " " && event.shiftKey);

        if (scrollsDown) {
          const now = performance.now();
          markUserInput();
          lastActiveScrollDownTsRef.current = now;
        } else if (scrollsUp) {
          markUserInput();
          autoFollowRef.current = false;
          userScrolledUpRef.current = true;
          setShowFollowButton(true);
        }

        onKeyDown?.(event);
      };

      const handleTouchStart: React.TouchEventHandler<HTMLDivElement> = (
        event,
      ) => {
        pointerDownRef.current = true;
        markUserInput();
        restProps.onTouchStart?.(event);
      };

      const handleTouchMove: React.TouchEventHandler<HTMLDivElement> = (
        event,
      ) => {
        markUserInput();
        restProps.onTouchMove?.(event);
      };

      const handleTouchEnd: React.TouchEventHandler<HTMLDivElement> = (
        event,
      ) => {
        pointerDownRef.current = false;
        restProps.onTouchEnd?.(event);
      };

      const handleTouchCancel: React.TouchEventHandler<HTMLDivElement> = (
        event,
      ) => {
        pointerDownRef.current = false;
        restProps.onTouchCancel?.(event);
      };

      const handlePointerDown: React.PointerEventHandler<HTMLDivElement> = (
        event,
      ) => {
        pointerDownRef.current = true;
        markUserInput();
        restProps.onPointerDown?.(event);
      };

      const handlePointerUp: React.PointerEventHandler<HTMLDivElement> = (
        event,
      ) => {
        pointerDownRef.current = false;
        restProps.onPointerUp?.(event);
      };

      const handlePointerCancel: React.PointerEventHandler<HTMLDivElement> = (
        event,
      ) => {
        pointerDownRef.current = false;
        restProps.onPointerCancel?.(event);
      };

      const handlePointerLeave: React.PointerEventHandler<HTMLDivElement> = (
        event,
      ) => {
        pointerDownRef.current = false;
        restProps.onPointerLeave?.(event);
      };

      const handleScroll: React.UIEventHandler<HTMLDivElement> = (event) => {
        const nextScrollTop = event.currentTarget.scrollTop;
        const now = performance.now();
        const recentUserIntent =
          pointerDownRef.current ||
          now - lastUserInputTsRef.current < SCROLL_INTENT_MS;
        const isSuppressedPassiveCorrection =
          now < suppressPassiveScrollUntilRef.current && !recentUserIntent;
        const isPassiveAdjustment =
          isSuppressedPassiveCorrection || !recentUserIntent;
        // Detect upward scroll as a safety net (keyboard, scrollbar drag,
        // touch, etc. — onWheel already covers mouse/trackpad). Use a +1px
        // tolerance to ignore sub-pixel Virtuoso measurement jitter.
        if (
          !isPassiveAdjustment &&
          nextScrollTop + 1 < lastScrollTopRef.current
        ) {
          autoFollowRef.current = false;
          userScrolledUpRef.current = true;
          setShowFollowButton(true);
          markUserInput();
        } else if (nextScrollTop > lastScrollTopRef.current + 1) {
          if (recentUserIntent) {
            lastActiveScrollDownTsRef.current = now;
          }
        }
        // NOTE: We intentionally do NOT infer "user scrolling down" from
        // scrollTop increases. Virtuoso's internal offset corrections during
        // item remeasurement can increase scrollTop without any user gesture,
        // and mistaking those for active scrolling would re-arm auto-follow
        // and cause visible scroll jumps while reading.
        lastScrollTopRef.current = nextScrollTop;
        onScroll?.(event);
      };

      return (
        <div
          ref={ref}
          style={{
            ...style,
            overflowY: "auto",
            overflowX: "hidden",
          }}
          data-testid="chat-virtuoso-scroller"
          className={classNames(styles.virtuosoScroller, className)}
          {...restProps}
          onWheel={handleWheel}
          onKeyDown={handleKeyDown}
          onTouchStart={handleTouchStart}
          onTouchMove={handleTouchMove}
          onTouchEnd={handleTouchEnd}
          onTouchCancel={handleTouchCancel}
          onPointerDown={handlePointerDown}
          onPointerUp={handlePointerUp}
          onPointerCancel={handlePointerCancel}
          onPointerLeave={handlePointerLeave}
          onScroll={handleScroll}
        >
          {children}
        </div>
      );
    });
    return ScrollerComponent;
  }, [markUserInput]);

  const List = useMemo(() => {
    const ListComponent = React.forwardRef<
      HTMLDivElement,
      React.HTMLAttributes<HTMLDivElement>
    >(function VirtuosoList({ children, style, ...props }, ref) {
      return (
        <Flex
          ref={ref}
          direction="column"
          className={styles.content}
          p="2"
          gap="1"
          style={style}
          {...props}
        >
          {children}
        </Flex>
      );
    });
    return ListComponent;
  }, []);

  const Footer = useCallback(
    () => (
      <>
        {footer}
        <Box style={{ height: 80 }} />
      </>
    ),
    [footer],
  );

  const components = useMemo(
    () => ({ Scroller, List, Footer }),
    [Scroller, List, Footer],
  );

  const viewportPadding = useMemo(
    () =>
      isStreaming ? { top: 800, bottom: 1200 } : { top: 800, bottom: 1000 },
    [isStreaming],
  );

  return (
    <Box
      ref={wrapperRef}
      style={{ flexGrow: 1, height: "100%", position: "relative" }}
      data-testid="chat-virtualized-list-wrapper"
    >
      {hasMeasuredHeight && (
        <Virtuoso
          ref={virtuosoRef}
          data={items}
          computeItemKey={computeItemKey}
          itemContent={itemContent}
          components={components}
          atBottomStateChange={handleAtBottomChange}
          followOutput={followOutput}
          initialTopMostItemIndex={
            initialScrollIndex !== undefined
              ? { index: initialScrollIndex, align: "end" }
              : undefined
          }
          atBottomThreshold={20}
          increaseViewportBy={viewportPadding}
          defaultItemHeight={DEFAULT_ITEM_HEIGHT}
          minOverscanItemCount={VIRTUOSO_MIN_OVERSCAN_ITEM_COUNT}
          overscan={VIRTUOSO_OVERSCAN}
          skipAnimationFrameInResizeObserver={true}
        />
      )}
      {showFollowButton && <ScrollToBottomButton onClick={handleFollowClick} />}
    </Box>
  );
}

VirtualizedChatList.displayName = "VirtualizedChatList";
