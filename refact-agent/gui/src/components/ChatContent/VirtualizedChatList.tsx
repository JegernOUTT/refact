/* eslint-disable react/prop-types */
import React, { useCallback, useRef, useState, useMemo } from "react";
import { Virtuoso, VirtuosoHandle } from "react-virtuoso";
import { Flex, Container, Box } from "@radix-ui/themes";
import { ScrollToBottomButton } from "../ScrollArea/ScrollToBottomButton";
import styles from "./ChatContent.module.css";

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
  // Timestamp of the last wheel/touch event that scrolled downward.
  // Used to distinguish real user scroll-down from Virtuoso measurement
  // adjustments that passively change scrollTop.
  const lastActiveScrollDownTsRef = useRef(0);

  const handleAtBottomChange = useCallback((bottom: boolean) => {
    if (bottom && userScrolledUpRef.current) {
      // Only re-arm auto-follow if the user recently performed an active
      // scroll-down gesture (wheel or touch). Virtuoso measurement
      // adjustments can passively shift the scroll position into the
      // atBottomThreshold — that must NOT re-arm follow.
      const recentActiveScroll =
        performance.now() - lastActiveScrollDownTsRef.current < 500;
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
    virtuosoRef.current?.scrollToIndex({
      index: items.length - 1,
      align: "end",
      behavior: "smooth",
    });
  }, [items.length]);

  const followOutput = useCallback(
    (isAtBottom: boolean) => {
      if (!isStreaming) return false;
      if (userScrolledUpRef.current) return false;
      if (isAtBottom && autoFollowRef.current) {
        return "auto";
      }
      return false;
    },
    [isStreaming],
  );

  const computeItemKey = useCallback((_index: number, item: T) => item.key, []);

  const itemContent = useCallback(
    (_index: number, item: T) => <Container>{renderItem(item)}</Container>,
    [renderItem],
  );

  const Scroller = useMemo(() => {
    const ScrollerComponent = React.forwardRef<
      HTMLDivElement,
      React.HTMLAttributes<HTMLDivElement>
    >(function VirtuosoScroller(props, ref) {
      const { children, style, onWheel, onScroll, ...restProps } = props;
      const handleWheel: React.WheelEventHandler<HTMLDivElement> = (event) => {
        if (event.deltaY < 0) {
          autoFollowRef.current = false;
          userScrolledUpRef.current = true;
          setShowFollowButton(true);
        } else if (event.deltaY > 0) {
          lastActiveScrollDownTsRef.current = performance.now();
        }
        onWheel?.(event);
      };

      const handleScroll: React.UIEventHandler<HTMLDivElement> = (event) => {
        const nextScrollTop = event.currentTarget.scrollTop;
        // Detect upward scroll as a safety net (keyboard, scrollbar drag,
        // touch, etc. — onWheel already covers mouse/trackpad).  Use a +1px
        // tolerance to ignore sub-pixel Virtuoso measurement jitter.
        if (nextScrollTop + 1 < lastScrollTopRef.current) {
          autoFollowRef.current = false;
          userScrolledUpRef.current = true;
          setShowFollowButton(true);
        }
        // NOTE: We intentionally do NOT infer "user scrolling down" from
        // scrollTop increases.  Virtuoso's internal offset corrections during
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
          className={styles.virtuosoScroller}
          {...restProps}
          onWheel={handleWheel}
          onScroll={handleScroll}
        >
          {children}
        </div>
      );
    });
    return ScrollerComponent;
  }, []);

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
      isStreaming ? { top: 1600, bottom: 2400 } : { top: 3200, bottom: 4400 },
    [isStreaming],
  );

  return (
    <Box style={{ flexGrow: 1, height: "100%", position: "relative" }}>
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
      />
      {showFollowButton && <ScrollToBottomButton onClick={handleFollowClick} />}
    </Box>
  );
}

VirtualizedChatList.displayName = "VirtualizedChatList";
