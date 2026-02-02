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
};

export function VirtualizedChatList<T extends { key: string }>({
  items,
  renderItem,
  initialScrollIndex,
  footer,
}: VirtualizedChatListProps<T>) {
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const [atBottom, setAtBottom] = useState(true);
  const [followMode, setFollowMode] = useState(true);

  const handleAtBottomChange = useCallback((bottom: boolean) => {
    setAtBottom(bottom);
    if (bottom) {
      setFollowMode(true);
    } else {
      setFollowMode(false);
    }
  }, []);

  const handleFollowClick = useCallback(() => {
    setFollowMode(true);
    virtuosoRef.current?.scrollToIndex({
      index: items.length - 1,
      align: "end",
      behavior: "smooth",
    });
  }, [items.length]);

  const followOutput = useCallback(
    (isAtBottom: boolean) => {
      if (followMode || isAtBottom) {
        return "smooth";
      }
      return false;
    },
    [followMode],
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
      // eslint-disable-next-line react/prop-types
    >(function VirtuosoScroller({ children, style, ...props }, ref) {
      return (
        <div
          ref={ref}
          style={{
            ...style,
            overflowY: "auto",
            overflowX: "hidden",
          }}
          className={styles.virtuosoScroller}
          {...props}
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
      // eslint-disable-next-line react/prop-types
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

  const showFollowButton = !atBottom && !followMode;

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
        atBottomThreshold={50}
        increaseViewportBy={{ top: 200, bottom: 200 }}
      />
      {showFollowButton && <ScrollToBottomButton onClick={handleFollowClick} />}
    </Box>
  );
}

VirtualizedChatList.displayName = "VirtualizedChatList";
