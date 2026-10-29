import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
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
  "--bb-bg"?: "var(--rf-surface-overlay)";
  "--bb-ink"?: "var(--rf-color-fg)";
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

  const anchorRef = useRef<HTMLDivElement | null>(null);
  const anchorKey = JSON.stringify(anchorStyle);
  const [edgeShift, setEdgeShift] = useState({ x: 0, y: 0 });

  useLayoutEffect(() => {
    const el = anchorRef.current;
    if (!el || !visible) {
      setEdgeShift({ x: 0, y: 0 });
      return;
    }
    const clampToViewport = () => {
      const margin = 8;
      el.style.translate = "0px 0px";
      const rect = el.getBoundingClientRect();
      const bounds = el
        .closest<HTMLElement>("[data-buddy-scene-bounds]")
        ?.getBoundingClientRect();
      const minLeft = Math.max(bounds?.left ?? 0, 0) + margin;
      const maxRight =
        Math.min(bounds?.right ?? window.innerWidth, window.innerWidth) -
        margin;
      const minTop = (bounds?.top ?? 0) + margin;
      const maxBottom = (bounds?.bottom ?? window.innerHeight) - margin;
      let shiftY = 0;
      if (bounds && rect.height > 0) {
        if (rect.top < minTop) shiftY = minTop - rect.top;
        else if (rect.bottom > maxBottom) shiftY = maxBottom - rect.bottom;
      }
      let shift = 0;
      if (rect.width > 0) {
        if (rect.width > maxRight - minLeft) {
          shift = minLeft - rect.left;
        } else if (rect.left < minLeft) {
          shift = minLeft - rect.left;
        } else if (rect.right > maxRight) {
          shift = maxRight - rect.right;
        }
      }
      el.style.translate = "";
      const scale = el.offsetWidth > 0 ? rect.width / el.offsetWidth : 1;
      const unscale = (value: number) =>
        Math.round(scale > 0 ? value / scale : value);
      setEdgeShift({ x: unscale(shift), y: unscale(shiftY) });
    };
    clampToViewport();
    const scene = el.closest<HTMLElement>("[data-buddy-scene-bounds]");
    window.addEventListener("resize", clampToViewport);
    scene?.addEventListener("transitionend", clampToViewport);
    return () => {
      window.removeEventListener("resize", clampToViewport);
      scene?.removeEventListener("transitionend", clampToViewport);
    };
  }, [textKey, position, compact, visible, width, maxWidth, anchorKey]);

  const style: BubbleVars = {
    ...anchorStyle,
    width,
    maxWidth: `min(${maxWidth}, calc(100dvw - 16px), var(--buddy-bubble-max, 100dvw))`,
    whiteSpace,
    overflowWrap: "break-word",
    pointerEvents: hasControls && !closing ? "auto" : "none",
    visibility: visible ? "visible" : "hidden",
    opacity,
    "--bb-bg": "var(--rf-surface-overlay)",
    "--bb-ink": "var(--rf-color-fg)",
    "--bb-border": palette.dark,
    "--bb-accent": palette.body,
    "--bb-accent-soft": `${palette.body}55`,
    translate:
      edgeShift.x === 0 && edgeShift.y === 0
        ? undefined
        : `${edgeShift.x}px ${edgeShift.y}px`,
  };

  return (
    <div
      ref={anchorRef}
      data-testid="buddy-speech-bubble"
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
