import React, { useMemo } from "react";
import {
  DashboardBadge as Badge,
  DashboardFlex as Flex,
  DashboardHoverCard as HoverCard,
  DashboardText as Text,
  dashboardToneFromMode,
} from "../DashboardPrimitives";
import type { HistoryTreeNode } from "../../../History/historySlice";
import type { DashboardBreakpoint } from "../../types";
import { buildDotTrail, type TrailDot } from "./buildDotTrail";
import styles from "./DotTrail.module.css";

type DotTrailProps = {
  node: HistoryTreeNode;
  breakpoint: DashboardBreakpoint;
  onDotClick?: (chatId: string) => void;
};

const DOT_SIZE: Record<DashboardBreakpoint, number> = {
  narrow: 10,
  medium: 11,
  wide: 12,
};

function buildNodeMap(
  node: HistoryTreeNode,
  map: Map<string, HistoryTreeNode>,
): void {
  map.set(node.id, node);
  for (const child of [...node.children, ...node.bubbleChildren]) {
    buildNodeMap(child, map);
  }
}

function DotHoverContent({
  dot,
  node,
}: {
  dot: TrailDot;
  node: HistoryTreeNode;
}) {
  const messageCount = node.message_count ?? 0;
  return (
    <Flex direction="column" gap="2">
      <Text size="2" weight="bold" truncate>
        {node.title || "New Chat"}
      </Text>

      {dot.label && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Type:
          </Text>
          <Text size="1">{dot.label}</Text>
        </Flex>
      )}

      {node.model && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Model:
          </Text>
          <Text size="1">{node.model}</Text>
        </Flex>
      )}

      {node.mode && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Mode:
          </Text>
          <Badge size="1" tone={dashboardToneFromMode(node.mode)}>
            {node.mode}
          </Badge>
        </Flex>
      )}

      {messageCount > 0 && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Messages:
          </Text>
          <Text size="1">{messageCount}</Text>
        </Flex>
      )}

      {((node.total_lines_added ?? 0) > 0 ||
        (node.total_lines_removed ?? 0) > 0) && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Changes:
          </Text>
          {(node.total_lines_added ?? 0) > 0 && (
            <Text size="1" tone="success">
              +{node.total_lines_added}
            </Text>
          )}
          {(node.total_lines_removed ?? 0) > 0 && (
            <Text size="1" tone="danger">
              −{node.total_lines_removed}
            </Text>
          )}
        </Flex>
      )}

      {node.session_state && node.session_state !== "idle" && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Status:
          </Text>
          <Text size="1">{node.session_state}</Text>
        </Flex>
      )}

      <Text size="1" tone="muted">
        {new Date(node.createdAt).toLocaleString()}
      </Text>
    </Flex>
  );
}

export const DotTrail: React.FC<DotTrailProps> = ({
  node,
  breakpoint,
  onDotClick,
}) => {
  const maxDots =
    breakpoint === "narrow" ? 6 : breakpoint === "medium" ? 8 : 10;

  const nodeMap = useMemo(() => {
    const map = new Map<string, HistoryTreeNode>();
    buildNodeMap(node, map);
    return map;
  }, [node]);

  const dots = useMemo(() => buildDotTrail(node, maxDots), [node, maxDots]);

  if (dots.length === 0) return null;

  const dotSize = DOT_SIZE[breakpoint];

  return (
    <Flex
      align="center"
      gap="1"
      className={styles.trail}
      role="group"
      aria-label="Thread trail"
    >
      {dots.map((dot, i) => {
        const dotNode = nodeMap.get(dot.chatId) ?? node;
        const size = dot.hasBranch ? dotSize + 3 : dotSize;

        return (
          <React.Fragment key={dot.id}>
            {i > 0 && breakpoint !== "narrow" && (
              <div className={styles.connector} />
            )}
            <HoverCard.Root>
              <HoverCard.Trigger>
                <span
                  role={onDotClick ? "button" : undefined}
                  className={`  rf-pressable`}
                  style={{
                    width: size,
                    height: size,
                    cursor: onDotClick ? "pointer" : "default",
                  }}
                  onClick={
                    onDotClick
                      ? (e: React.MouseEvent) => {
                          e.stopPropagation();
                          onDotClick(dot.chatId);
                        }
                      : undefined
                  }
                  onKeyDown={
                    onDotClick
                      ? (e: React.KeyboardEvent) => {
                          if (e.key === "Enter" || e.key === " ") {
                            e.preventDefault();
                            e.stopPropagation();
                            onDotClick(dot.chatId);
                          }
                        }
                      : undefined
                  }
                  tabIndex={onDotClick ? 0 : -1}
                  aria-label={dotNode.title || "Chat"}
                />
              </HoverCard.Trigger>
              <HoverCard.Content
                className={styles.hoverCard}
              >
                <DotHoverContent dot={dot} node={dotNode} />
              </HoverCard.Content>
            </HoverCard.Root>
          </React.Fragment>
        );
      })}
    </Flex>
  );
};
