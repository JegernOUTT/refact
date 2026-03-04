import React, { useMemo } from "react";
import { Flex, Tooltip } from "@radix-ui/themes";
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
  for (const child of node.children) {
    buildNodeMap(child, map);
  }
}

function getDotTooltip(dot: TrailDot, node: HistoryTreeNode): string {
  const parts: string[] = [];
  // eslint-disable-next-line @typescript-eslint/prefer-nullish-coalescing -- intentional: fall through empty strings
  const title = node.title || dot.label || dot.type;
  parts.push(title);
  if (node.mode) parts.push(`Mode: ${node.mode}`);
  if (node.model) parts.push(`Model: ${node.model}`);
  if ((node.message_count ?? 0) > 0) parts.push(`${node.message_count} msgs`);
  if (node.session_state && node.session_state !== "idle") {
    parts.push(`Status: ${node.session_state}`);
  }
  return parts.join(" · ");
}

export const DotTrail: React.FC<DotTrailProps> = ({
  node,
  breakpoint,
  onDotClick,
}) => {
  const maxDots = breakpoint === "narrow" ? 6 : breakpoint === "medium" ? 8 : 10;

  const nodeMap = useMemo(() => {
    const map = new Map<string, HistoryTreeNode>();
    buildNodeMap(node, map);
    return map;
  }, [node]);

  const dots = useMemo(() => buildDotTrail(node, maxDots), [node, maxDots]);

  if (dots.length <= 1) return null;

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
        const tooltip = getDotTooltip(dot, dotNode);
        const size = dot.hasBranch ? dotSize + 3 : dotSize;

        return (
          <React.Fragment key={dot.id}>
            {i > 0 && breakpoint !== "narrow" && (
              <div className={styles.connector} />
            )}
            <Tooltip content={tooltip}>
              <button
                type="button"
                className={`${styles.dot} ${styles[dot.type]}`}
                style={{
                  width: size,
                  height: size,
                  cursor: onDotClick ? "pointer" : "default",
                }}
                onClick={onDotClick ? (e: React.MouseEvent) => {
                  e.stopPropagation();
                  onDotClick(dot.chatId);
                } : undefined}
                tabIndex={onDotClick ? 0 : -1}
                aria-label={tooltip}
              />
            </Tooltip>
          </React.Fragment>
        );
      })}
    </Flex>
  );
};
