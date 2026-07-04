import React, { useEffect, useState } from "react";
import classNames from "classnames";
import type { BubblePosition, BuddyControl, Palette } from "./types";
import type { BuddySpeechStyle } from "./buddySpeech";
import styles from "./BuddySpeechBubble.module.css";

export type BuddySpeechExitKind = "natural" | "accept" | "dismiss";

export interface BuddySpeechBubbleProps {
  text: string;
  textKey: number;
  enterKey?: number;
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
  bubbleStyle?: BuddySpeechStyle;
  closing?: boolean;
  exitKind?: BuddySpeechExitKind;
  media?: React.ReactNode;
  controls?: BuddyControl[];
  onControlClick?: (control: BuddyControl) => void | Promise<void>;
}

type BubbleVars = React.CSSProperties & {
  "--bb-bg"?: string;
  "--bb-ink"?: string;
  "--bb-border"?: string;
  "--bb-accent"?: string;
  "--bb-accent-soft"?: string;
};

const SING_NOTES = ["♪", "♫", "♪"] as const;

function isDismissControl(control: BuddyControl): boolean {
  return (
    control.action === "dismiss" ||
    control.action === "dismiss_speech" ||
    control.action === "dismiss_runtime_event" ||
    control.action === "dismiss_suggestion"
  );
}

export const BuddySpeechBubble: React.FC<BuddySpeechBubbleProps> = ({
  text,
  textKey,
  enterKey = 0,
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
  bubbleStyle = "say",
  closing = false,
  exitKind = "natural",
  media,
  controls,
  onControlClick,
}) => {
  const hasControls = (controls?.length ?? 0) > 0;
  const [clickedId, setClickedId] = useState<string | null>(null);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [controlError, setControlError] = useState<string | null>(null);

  useEffect(() => {
    setClickedId(null);
    setPendingId(null);
    setControlError(null);
  }, [textKey]);

  const style: BubbleVars = {
    ...anchorStyle,
    width,
    maxWidth,
    whiteSpace,
    overflowWrap: "break-word",
    pointerEvents: hasControls && !closing ? "auto" : "none",
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
      data-style={bubbleStyle}
      data-closing={String(closing)}
      data-exit-kind={exitKind}
      className={styles.anchor}
      style={style}
    >
      <div key={enterKey} className={classNames(styles.skin)}>
        {bubbleStyle === "think" ? (
          <>
            <div className={styles.thinkTailLarge} />
            <div className={styles.thinkTailMedium} />
            <div className={styles.thinkTailSmall} />
          </>
        ) : (
          <>
            <div className={styles.tailOuter} />
            <div className={styles.tailInner} />
          </>
        )}
        {bubbleStyle === "sing" ? (
          <div className={styles.singNotes} aria-hidden>
            {SING_NOTES.map((note, index) => (
              <span
                key={index}
                className={styles.singNote}
                data-note-index={index}
              >
                {note}
              </span>
            ))}
          </div>
        ) : null}
        {bubbleStyle === "alert" ? (
          <div className={styles.alertRing} aria-hidden />
        ) : null}
        <div key={textKey} className={styles.content}>
          {intent ? <span className={styles.intent}>{intent}</span> : null}
          <span>{text}</span>
          {media ? <div className={styles.media}>{media}</div> : null}
          {hasControls ? (
            <div className={styles.controls}>
              {controls?.map((ctrl) => (
                <button
                  key={ctrl.id}
                  type="button"
                  data-control-style={ctrl.style}
                  data-clicked={String(clickedId === ctrl.id)}
                  data-pending={String(pendingId === ctrl.id)}
                  data-faded={String(
                    clickedId !== null && clickedId !== ctrl.id,
                  )}
                  className={styles.controlButton}
                  disabled={pendingId !== null}
                  aria-label={
                    controlError && clickedId === ctrl.id
                      ? controlError
                      : ctrl.label
                  }
                  title={
                    controlError && clickedId === ctrl.id
                      ? controlError
                      : undefined
                  }
                  onClick={(event) => {
                    event.stopPropagation();
                    if (pendingId !== null) return;
                    setClickedId(ctrl.id);
                    setControlError(null);
                    if (!isDismissControl(ctrl)) {
                      void onControlClick?.(ctrl);
                      return;
                    }
                    setPendingId(ctrl.id);
                    void Promise.resolve(onControlClick?.(ctrl))
                      .catch(() => {
                        setClickedId(null);
                        setControlError("Could not dismiss. Try again.");
                      })
                      .finally(() => setPendingId(null));
                  }}
                >
                  {ctrl.label}
                </button>
              ))}
            </div>
          ) : null}
          {controlError ? (
            <span title={controlError}>{controlError}</span>
          ) : null}
        </div>
      </div>
    </div>
  );
};
