import React from "react";
import classNames from "classnames";
import { useAppSelector } from "../../../../hooks/useAppSelector";
import type { FamilyChild } from "../../streamSelectors";
import { selectFamilyChildren } from "../../streamSelectors";
import { formatRelativeTime } from "./streamRowUtils";
import styles from "./Stream.module.css";

const INDENT_CLASSES = [styles.i0, styles.i1, styles.i2, styles.i3, styles.i4];

/** Selector depth is 1-based below the root; top-level children sit at indent 0
    so their elbow line drops exactly under the parent row's tile centre. */
function indentClass(depth: number): string {
  return INDENT_CLASSES[Math.min(Math.max(depth - 1, 0), 4)];
}

function dotClass(status: FamilyChild["status"]): string {
  switch (status) {
    case "streaming":
    case "working":
      return styles.railDotLive;
    case "done":
      return styles.railDotDone;
    case "failed":
      return styles.railDotFailed;
    default:
      return "";
  }
}

function metaLabel(child: FamilyChild): string | null {
  const parts: string[] = [];
  if (child.messageCount != null) parts.push(`${child.messageCount} msgs`);
  if (child.status === "streaming") parts.push("streaming");
  else if (child.status === "working") parts.push("working");
  return parts.length > 0 ? parts.join(" · ") : null;
}

export type ThreadRailProps = {
  rootId: string;
  onOpenChat: (id: string) => void;
};

export const ThreadRail: React.FC<ThreadRailProps> = ({
  rootId,
  onOpenChat,
}) => {
  const children = useAppSelector((state) =>
    selectFamilyChildren(state, rootId),
  );

  if (children.length === 0) return null;

  return (
    <div className={styles.rail} data-testid={`stream-rail-${rootId}`}>
      {children.map((child, index) => {
        const isLast =
          index === children.length - 1 ||
          children[index + 1].depth < child.depth;
        const isLive =
          child.status === "streaming" || child.status === "working";
        const meta = metaLabel(child);

        return (
          <div
            key={child.id}
            role="button"
            tabIndex={0}
            data-testid={`stream-rail-row-${child.id}`}
            className={classNames(styles.railRow, indentClass(child.depth))}
            onClick={() => onOpenChat(child.id)}
            onKeyDown={(event) => {
              if (event.key !== "Enter" && event.key !== " ") return;
              event.preventDefault();
              onOpenChat(child.id);
            }}
          >
            <span
              aria-hidden="true"
              className={classNames(
                styles.railElbow,
                isLast && styles.railElbowLast,
              )}
            />
            <span
              className={classNames(styles.railDot, dotClass(child.status))}
            />
            <span className={styles.railTitleWrap}>
              <span
                className={classNames(
                  styles.railTitle,
                  isLive && styles.railTitleLive,
                )}
              >
                {child.title}
              </span>
              {meta !== null && <span className={styles.railMeta}>{meta}</span>}
            </span>
            <span className={styles.railLinkType}>
              {child.linkType === "branch" ? "Branch" : "Subchat"}
            </span>
            <span aria-hidden="true" />
            <span className={styles.railTime}>
              {formatRelativeTime(child.updatedAtMs)}
            </span>
          </div>
        );
      })}
    </div>
  );
};
