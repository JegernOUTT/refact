import React from "react";
import classNames from "classnames";
import type { BubblePosition, BuddyControl, Palette } from "./types";
import styles from "./BuddySpeechBubble.module.css";

export interface BuddySpeechBubbleProps {
  text: string;
  textKey: number;
  position: BubblePosition;
  palette: Palette;
  visible: boolean;
  opacity: number;
  compact: boolean;
  width: string;
  maxWidth: string;
  whiteSpace: React.CSSProperties["whiteSpace"];
  anchorStyle: React.CSSProperties;
  intent?: string;
  controls?: BuddyControl[];
  onControlClick?: (control: BuddyControl) => void;
}

type BubbleVars = React.CSSProperties & {
  "--bb-bg"?: string;
  "--bb-ink"?: string;
  "--bb-border"?: string;
  "--bb-accent"?: string;
  "--bb-accent-soft"?: string;
};

export const BuddySpeechBubble: React.FC<BuddySpeechBubbleProps> = ({
  text,
  textKey,
  position,
  palette,
  visible,
  opacity,
  compact,
  width,
  maxWidth,
  whiteSpace,
  anchorStyle,
  intent,
  controls,
  onControlClick,
}) => {
  const hasControls = (controls?.length ?? 0) > 0;
  const style: BubbleVars = {
    ...anchorStyle,
    width,
    maxWidth,
    whiteSpace,
    overflowWrap: "break-word",
    pointerEvents: hasControls ? "auto" : "none",
    visibility: visible ? "visible" : "hidden",
    opacity,
    "--bb-bg": "#FBF6EA",
    "--bb-ink": "#33302A",
    "--bb-border": palette.dark,
    "--bb-accent": palette.body,
    "--bb-accent-soft": `${palette.body}55`,
  };

  return (
    <div
      data-bubble-position={position}
      data-compact={String(compact)}
      className={styles.anchor}
      style={style}
    >
      <div key={textKey} className={classNames(styles.skin)}>
        <div className={styles.tailOuter} />
        <div className={styles.tailInner} />
        <div className={styles.content}>
          {intent ? <span className={styles.intent}>{intent}</span> : null}
          <span>{text}</span>
          {hasControls ? (
            <div className={styles.controls}>
              {controls?.map((ctrl) => (
                <button
                  key={ctrl.id}
                  type="button"
                  data-control-style={ctrl.style}
                  className={styles.controlButton}
                  onClick={(event) => {
                    event.stopPropagation();
                    onControlClick?.(ctrl);
                  }}
                >
                  {ctrl.label}
                </button>
              ))}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
};
