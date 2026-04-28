import React, { useRef, useEffect, useCallback } from "react";
import { createInitialAnimState } from "./state";
import { renderFrame } from "./canvas/render";
import {
  stepAnimFrame,
  triggerSignalAnimation,
  handlePet,
} from "./canvas/animLoop";
import {
  CANVAS_SIZE,
  CANVAS_CENTER_X,
  CANVAS_CENTER_Y,
  STAGE_SIZES,
  PALETTES,
} from "./constants";
import type {
  BuddyCanvasProps,
  BuddyAnimState,
  BuddySemanticState,
  BuddyEvent,
} from "./types";

const BUBBLE_STYLES: Record<
  string,
  {
    container: React.CSSProperties;
    tail: React.CSSProperties;
  }
> = {
  above: {
    container: {
      bottom: "70%",
      left: "50%",
      transform: "translateX(-50%)",
    },
    tail: {
      top: "100%",
      left: "50%",
      transform: "translateX(-50%)",
      borderLeft: "7px solid transparent",
      borderRight: "7px solid transparent",
      /* borderTop set dynamically via palette */
    },
  },
  left: {
    container: {
      right: "100%",
      top: "10%",
      marginRight: "8px",
    },
    tail: {
      left: "100%",
      top: "50%",
      transform: "translateY(-50%)",
      borderTop: "7px solid transparent",
      borderBottom: "7px solid transparent",
      /* borderLeft set dynamically via palette */
    },
  },
  right: {
    container: {
      left: "100%",
      top: "10%",
      marginLeft: "8px",
    },
    tail: {
      right: "100%",
      top: "50%",
      transform: "translateY(-50%)",
      borderTop: "7px solid transparent",
      borderBottom: "7px solid transparent",
      /* borderRight set dynamically via palette */
    },
  },
};

export const BuddyCanvas: React.FC<BuddyCanvasProps> = ({
  state,
  onEvent,
  displaySize = 512,
  className,
  style,
  speechOverride,
  speechControls,
  onSpeechControlClick,
  bubblePosition = "above",
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const animRef = useRef<BuddyAnimState>(createInitialAnimState());
  const semanticRef = useRef<BuddySemanticState>(state);
  const prevSignalTimeRef = useRef<number>(0);
  const frameIdRef = useRef<number>(0);
  const bubbleRef = useRef<HTMLDivElement>(null);
  const bubbleTextRef = useRef<string>("");
  const speechOverrideRef = useRef<string | null | undefined>(speechOverride);

  useEffect(() => {
    speechOverrideRef.current = speechOverride;
  }, [speechOverride]);

  const palette = PALETTES[state.paletteIndex] ?? PALETTES[0];

  useEffect(() => {
    semanticRef.current = state;
  }, [state]);

  const emit = useCallback(
    (event: BuddyEvent) => {
      onEvent?.(event);
    },
    [onEvent],
  );

  useEffect(() => {
    const { lastSignalTime, lastSignalType } = state.activity;
    if (
      lastSignalTime !== prevSignalTimeRef.current &&
      lastSignalTime > 0 &&
      lastSignalType
    ) {
      prevSignalTimeRef.current = lastSignalTime;
      triggerSignalAnimation(animRef.current, lastSignalType, emit);
    }
  }, [state.activity, emit]);

  useEffect(() => {
    const loop = () => {
      const ctx = canvasRef.current?.getContext("2d");
      if (ctx) {
        const sem = semanticRef.current;
        stepAnimFrame(animRef.current, sem, emit);
        renderFrame(ctx, animRef.current, sem);
        if (bubbleRef.current) {
          const anim = animRef.current;
          const overrideText = speechOverrideRef.current ?? "";
          const rawText = overrideText || anim.statusText || "";
          // Cap speech-bubble text length so a runaway error message or
          // a long backend speech_text cannot blow out the hero layout.
          // 140 chars matches roughly two short lines at our default size.
          const text =
            rawText.length > 140
              ? `${rawText.slice(0, 137).trimEnd()}…`
              : rawText;
          const opacity = overrideText ? 1 : anim.statusOpacity;
          if (text !== bubbleTextRef.current) {
            bubbleTextRef.current = text;
            const span = bubbleRef.current
              .firstElementChild as HTMLElement | null;
            if (span) span.textContent = text;
          }
          if (opacity > 0.02 && text) {
            bubbleRef.current.style.opacity = String(Math.min(1, opacity));
            bubbleRef.current.style.visibility = "visible";
          } else {
            bubbleRef.current.style.opacity = "0";
            bubbleRef.current.style.visibility = "hidden";
          }
        }
      }
      frameIdRef.current = requestAnimationFrame(loop);
    };
    frameIdRef.current = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(frameIdRef.current);
  }, [emit]);

  const toCanvasCoords = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const rect = canvasRef.current?.getBoundingClientRect();
      if (!rect) return null;
      return {
        x: ((e.clientX - rect.left) / rect.width) * CANVAS_SIZE,
        y: ((e.clientY - rect.top) / rect.height) * CANVAS_SIZE,
        normX: ((e.clientX - rect.left) / rect.width) * 2 - 1,
        normY: ((e.clientY - rect.top) / rect.height) * 2 - 1,
      };
    },
    [],
  );

  const onMouseMove = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const coords = toCanvasCoords(e);
      if (!coords) return;
      const anim = animRef.current;
      anim.mouseSpeed = Math.sqrt(
        (coords.normX - anim.cursorTargetX) ** 2 +
          (coords.normY - anim.cursorTargetY) ** 2,
      );
      anim.cursorTargetX = coords.normX;
      anim.cursorTargetY = coords.normY;
      const stage = semanticRef.current.progress.stage;
      const [spriteW] = STAGE_SIZES[stage] ?? [28, 18];
      const buddyX = CANVAS_CENTER_X + anim.walkOffsetX;
      const dist = Math.sqrt(
        (coords.x - buddyX) ** 2 + (coords.y - CANVAS_CENTER_Y) ** 2,
      );
      anim.mouseOnBuddy = dist < spriteW / 2 + 4;
      const dx = (coords.normX * CANVAS_SIZE) / 2;
      const dy = (coords.normY * CANVAS_SIZE) / 2;
      anim.mouseProximity = Math.max(0, 1 - Math.sqrt(dx * dx + dy * dy) / 80);
      anim.mouseAngle = Math.atan2(coords.normY, coords.normX);
    },
    [toCanvasCoords],
  );

  const onMouseLeave = useCallback(() => {
    const anim = animRef.current;
    anim.mouseOnBuddy = false;
    anim.mouseProximity = 0;
    anim.mouseNearTimer = 0;
    anim.dragging = false;
  }, []);

  const onMouseDown = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const coords = toCanvasCoords(e);
      if (!coords) return;
      const stage = semanticRef.current.progress.stage;
      const [spriteW] = STAGE_SIZES[stage] ?? [28, 18];
      const hitRadius = spriteW / 2 + 4;
      const buddyX = CANVAS_CENTER_X + animRef.current.walkOffsetX;
      if (
        Math.sqrt(
          (coords.x - buddyX) ** 2 + (coords.y - CANVAS_CENTER_Y) ** 2,
        ) < hitRadius
      ) {
        animRef.current.dragging = true;
      }
    },
    [toCanvasCoords],
  );

  const onMouseUp = useCallback(() => {
    const anim = animRef.current;
    if (anim.dragging) {
      anim.dragging = false;
      anim.squashTargetX = 1.1;
      anim.squashTargetY = 0.9;
    }
  }, []);

  const onClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const coords = toCanvasCoords(e);
      if (!coords) return;
      const stage = semanticRef.current.progress.stage;
      handlePet(animRef.current, coords.x, coords.y, emit, stage);
    },
    [toCanvasCoords, emit],
  );

  return (
    <div
      className={className}
      style={{
        position: "relative",
        display: "inline-block",
        width: displaySize,
        height: displaySize,
        flexShrink: 0,
        ...style,
      }}
    >
      <canvas
        ref={canvasRef}
        width={CANVAS_SIZE}
        height={CANVAS_SIZE}
        style={{
          width: displaySize,
          height: displaySize,
          imageRendering: "pixelated",
          display: "block",
          cursor: "pointer",
        }}
        onMouseMove={onMouseMove}
        onMouseLeave={onMouseLeave}
        onMouseDown={onMouseDown}
        onMouseUp={onMouseUp}
        onClick={onClick}
      />
      {displaySize >= 100 &&
        (() => {
          const pos = BUBBLE_STYLES[bubblePosition];
          const tailColor: React.CSSProperties =
            bubblePosition === "left"
              ? { borderLeft: `7px solid ${palette.body}` }
              : bubblePosition === "right"
                ? { borderRight: `7px solid ${palette.body}` }
                : { borderTop: `7px solid ${palette.body}` };
          return (
            <div
              ref={bubbleRef}
              style={{
                position: "absolute",
                ...pos.container,
                background: "#0d0d16",
                border: `2px solid ${palette.body}`,
                borderRadius: "2px",
                padding: "4px 10px",
                fontSize: "10px",
                fontFamily: "'Courier New', Courier, monospace",
                fontWeight: 700,
                letterSpacing: "0.3px",
                whiteSpace: speechControls?.length ? "normal" : "nowrap",
                width: speechControls?.length ? "300px" : "max-content",
                pointerEvents: speechControls?.length ? "auto" : "none",
                color: palette.light,
                boxShadow: `3px 3px 0 ${palette.dark}, 0 0 0 3px #000`,
                zIndex: 5,
                visibility: "hidden",
                opacity: 0,
              }}
            >
              <span />
              {speechControls?.length ? (
                <div
                  style={{
                    display: "flex",
                    gap: "4px",
                    flexWrap: "wrap",
                    marginTop: "6px",
                  }}
                >
                  {speechControls.map((ctrl) => (
                    <button
                      key={ctrl.id}
                      onClick={(e) => {
                        e.stopPropagation();
                        onSpeechControlClick?.(ctrl);
                      }}
                      style={{
                        background:
                          ctrl.style === "primary"
                            ? palette.body
                            : "transparent",
                        border: `2px solid ${palette.body}`,
                        borderRadius: "2px",
                        color:
                          ctrl.style === "primary" ? "#0d0d16" : palette.light,
                        fontFamily: "'Courier New', Courier, monospace",
                        fontWeight: 700,
                        fontSize: "9px",
                        padding: "2px 6px",
                        cursor: "pointer",
                        letterSpacing: "0.3px",
                      }}
                    >
                      {ctrl.label}
                    </button>
                  ))}
                </div>
              ) : null}
              <div
                style={{
                  position: "absolute",
                  width: 0,
                  height: 0,
                  ...pos.tail,
                  ...tailColor,
                }}
              />
            </div>
          );
        })()}
    </div>
  );
};
