import React, { useRef, useEffect, useCallback, useState } from "react";
import { BuddySpeechBubble } from "./BuddySpeechBubble";
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
  BuddyEnvContext,
  BuddySemanticState,
  BuddyEvent,
  BubblePosition,
} from "./types";

const BUBBLE_POSITIONS: BubblePosition[] = ["top", "left", "right"];

function randomBubblePosition(previous?: BubblePosition): BubblePosition {
  const choices = previous
    ? BUBBLE_POSITIONS.filter((position) => position !== previous)
    : BUBBLE_POSITIONS;
  return choices[Math.floor(Math.random() * choices.length)] ?? "top";
}

function ellipsizeMiddle(text: string, maxLength: number): string {
  if (text.length <= maxLength) return text;
  const edgeLength = Math.floor((maxLength - 1) / 2);
  const start = text.slice(0, edgeLength).trimEnd();
  const end = text.slice(text.length - edgeLength).trimStart();
  return `${start}…${end}`;
}

interface BubbleView {
  text: string;
  textKey: number;
  position: BubblePosition;
  width:
    | "max-content"
    | "200px"
    | "220px"
    | "230px"
    | "240px"
    | "260px"
    | "270px"
    | "300px"
    | "330px";
  maxWidth: "220px" | "300px" | "min(460px, 72vw)";
  whiteSpace: React.CSSProperties["whiteSpace"];
  opacity: number;
  visible: boolean;
  walkOffsetPx: number;
}

function bubbleAnchorStyle(
  view: BubbleView,
  displaySize: number,
  stage: number,
  chatCompanion: boolean,
): React.CSSProperties {
  const k = displaySize / CANVAS_SIZE;
  const [spriteW, spriteH] = STAGE_SIZES[stage] ?? [28, 18];
  const walk = view.walkOffsetPx;
  if (view.position === "top") {
    const headTopPx = (CANVAS_CENTER_Y - 1.8 * (spriteH / 2 + 10)) * k;
    return {
      left: `calc(50% + ${walk}px)`,
      bottom: `${Math.round(displaySize - headTopPx + 4)}px`,
      transform: "translateX(-50%)",
    };
  }
  const faceTopPx = Math.round((CANVAS_CENTER_Y - 1.8 * (spriteH / 2 - 6)) * k);
  const sideEdgePx = Math.round(1.8 * (spriteW / 2 + 7) * k);
  if (view.position === "left") {
    return {
      right: chatCompanion
        ? `calc(78% - ${walk}px)`
        : `calc(50% + ${sideEdgePx}px - ${walk}px)`,
      top: `${faceTopPx}px`,
      transform: "translateY(-50%)",
    };
  }
  return {
    left: chatCompanion
      ? `calc(78% + ${walk}px)`
      : `calc(50% + ${sideEdgePx}px + ${walk}px)`,
    top: `${faceTopPx}px`,
    transform: "translateY(-50%)",
  };
}

export const BuddyCanvas: React.FC<BuddyCanvasProps> = ({
  state,
  onEvent,
  displaySize = 512,
  className,
  style,
  envContext,
  spritePointer = false,
  cursorBridgeRef,
  speechOverride,
  speechControls,
  speechIntent,
  onSpeechControlClick,
  bubblePosition = "top",
  randomizeBubblePosition = false,
  compactBubble: compactBubbleOverride = false,
  chatCompanionBubble: chatCompanionBubbleOverride = false,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const animRef = useRef<BuddyAnimState>(createInitialAnimState());
  const semanticRef = useRef<BuddySemanticState>(state);
  const envRef = useRef<BuddyEnvContext | null>(envContext ?? null);
  const prevSignalTimeRef = useRef<number>(0);
  const frameIdRef = useRef<number>(0);
  const [bubbleView, setBubbleView] = useState<BubbleView>(() => ({
    text: "",
    textKey: 0,
    position: bubblePosition,
    width: "max-content",
    maxWidth: "300px",
    whiteSpace: "nowrap",
    opacity: 0,
    visible: false,
    walkOffsetPx: 0,
  }));
  const bubbleViewRef = useRef<BubbleView>(bubbleView);
  const bubblePositionRef = useRef<BubblePosition>(bubblePosition);
  const speechOverrideRef = useRef<string | null | undefined>(speechOverride);
  const speechControlCount = speechControls?.length ?? 0;

  useEffect(() => {
    speechOverrideRef.current = speechOverride;
  }, [speechOverride]);

  useEffect(() => {
    bubbleViewRef.current = bubbleView;
  }, [bubbleView]);

  useEffect(() => {
    bubblePositionRef.current = bubblePosition;
    if (!randomizeBubblePosition) {
      setBubbleView((prev) => {
        if (prev.position === bubblePosition) return prev;
        return { ...prev, position: bubblePosition };
      });
    }
  }, [bubblePosition, randomizeBubblePosition]);

  const palette = PALETTES[state.paletteIndex] ?? PALETTES[0];

  useEffect(() => {
    semanticRef.current = state;
  }, [state]);

  useEffect(() => {
    envRef.current = envContext ?? null;
  }, [envContext]);

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
      triggerSignalAnimation(
        animRef.current,
        lastSignalType,
        emit,
        semanticRef.current,
      );
    }
  }, [state.activity, emit]);

  useEffect(() => {
    const loop = () => {
      if (document.hidden) {
        frameIdRef.current = requestAnimationFrame(loop);
        return;
      }

      const canvas = canvasRef.current;
      const ctx = canvas?.getContext("2d");
      if (canvas && ctx) {
        const pixelRatio = Math.min(
          2,
          Math.max(1, window.devicePixelRatio || 1),
        );
        const targetSize = Math.max(1, Math.round(displaySize * pixelRatio));
        if (canvas.width !== targetSize || canvas.height !== targetSize) {
          canvas.width = targetSize;
          canvas.height = targetSize;
        }
        const backingScale = targetSize / CANVAS_SIZE;
        const sem = semanticRef.current;
        stepAnimFrame(animRef.current, sem, emit, envRef.current);
        ctx.save();
        ctx.scale(backingScale, backingScale);
        renderFrame(ctx, animRef.current, sem, backingScale);
        ctx.restore();

        const anim = animRef.current;
        const previous = bubbleViewRef.current;
        const walkOffsetPx = Math.round(
          (anim.walkOffsetX / CANVAS_SIZE) * displaySize,
        );
        const compactBubble = compactBubbleOverride || displaySize <= 180;
        const chatCompanionBubble =
          chatCompanionBubbleOverride && displaySize <= 180;
        const overrideText = speechOverrideRef.current ?? "";
        const rawText = overrideText || anim.statusText || "";
        const text = ellipsizeMiddle(
          rawText,
          chatCompanionBubble ? 160 : compactBubble ? 120 : 170,
        );
        const opacity = overrideText ? 1 : anim.statusOpacity;
        const visible = opacity > 0.02 && text.length > 0;
        const hasControls = speechControlCount > 0;
        const isVeryLongText = text.length > 130;
        const isLongText = text.length > 72;
        const isMediumText = text.length > 34;
        const fixedWidth = hasControls || isLongText || chatCompanionBubble;
        const width: BubbleView["width"] = chatCompanionBubble
          ? isVeryLongText
            ? "330px"
            : isLongText || hasControls
              ? "300px"
              : isMediumText
                ? "260px"
                : "max-content"
          : compactBubble
            ? isLongText
              ? "220px"
              : hasControls
                ? "200px"
                : isMediumText
                  ? "200px"
                  : "max-content"
            : isVeryLongText
              ? "300px"
              : isLongText
                ? "270px"
                : hasControls
                  ? "230px"
                  : isMediumText
                    ? "200px"
                    : "max-content";
        const maxWidth: BubbleView["maxWidth"] = chatCompanionBubble
          ? "min(460px, 72vw)"
          : compactBubble
            ? "220px"
            : "300px";
        const whiteSpace: BubbleView["whiteSpace"] =
          fixedWidth || isMediumText ? "normal" : "nowrap";
        const previousFixedWidth =
          previous.width !== "max-content" &&
          previous.width !== "200px" &&
          previous.width !== "220px";
        const position =
          text !== previous.text || fixedWidth !== previousFixedWidth
            ? randomizeBubblePosition
              ? fixedWidth
                ? "top"
                : randomBubblePosition(previous.position)
              : bubblePositionRef.current
            : previous.position;
        const nextOpacity = visible ? Math.min(1, opacity) : 0;
        const opacityChanged = Math.abs(previous.opacity - nextOpacity) > 0.03;
        const nextView: BubbleView = {
          text,
          textKey:
            text !== previous.text && text.length > 0
              ? previous.textKey + 1
              : previous.textKey,
          position,
          width,
          maxWidth,
          whiteSpace,
          opacity: nextOpacity,
          visible,
          walkOffsetPx,
        };

        if (
          previous.text !== nextView.text ||
          previous.position !== nextView.position ||
          previous.width !== nextView.width ||
          previous.maxWidth !== nextView.maxWidth ||
          previous.whiteSpace !== nextView.whiteSpace ||
          previous.visible !== nextView.visible ||
          previous.walkOffsetPx !== nextView.walkOffsetPx ||
          opacityChanged
        ) {
          bubbleViewRef.current = nextView;
          setBubbleView(nextView);
        }
      }
      frameIdRef.current = requestAnimationFrame(loop);
    };
    frameIdRef.current = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(frameIdRef.current);
  }, [
    chatCompanionBubbleOverride,
    compactBubbleOverride,
    displaySize,
    emit,
    randomizeBubblePosition,
    speechControlCount,
  ]);

  const clientToCanvasCoords = useCallback(
    (clientX: number, clientY: number) => {
      const rect = canvasRef.current?.getBoundingClientRect();
      if (!rect || rect.width === 0 || rect.height === 0) return null;
      const clampNorm = (value: number) => Math.max(-1.6, Math.min(1.6, value));
      return {
        x: ((clientX - rect.left) / rect.width) * CANVAS_SIZE,
        y: ((clientY - rect.top) / rect.height) * CANVAS_SIZE,
        normX: clampNorm(((clientX - rect.left) / rect.width) * 2 - 1),
        normY: clampNorm(((clientY - rect.top) / rect.height) * 2 - 1),
      };
    },
    [],
  );

  const applyPointerMove = useCallback(
    (clientX: number, clientY: number) => {
      const coords = clientToCanvasCoords(clientX, clientY);
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
    [clientToCanvasCoords],
  );

  const onMouseMove = useCallback(
    (e: React.MouseEvent<HTMLElement>) => {
      applyPointerMove(e.clientX, e.clientY);
    },
    [applyPointerMove],
  );

  const onMouseLeave = useCallback(() => {
    const anim = animRef.current;
    anim.mouseOnBuddy = false;
    anim.mouseProximity = 0;
    anim.mouseNearTimer = 0;
    anim.dragging = false;
  }, []);

  useEffect(() => {
    if (!cursorBridgeRef) return;
    cursorBridgeRef.current = {
      move: applyPointerMove,
      leave: onMouseLeave,
    };
    return () => {
      cursorBridgeRef.current = null;
    };
  }, [applyPointerMove, cursorBridgeRef, onMouseLeave]);

  const onMouseDown = useCallback(
    (e: React.MouseEvent<HTMLElement>) => {
      const coords = clientToCanvasCoords(e.clientX, e.clientY);
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
    [clientToCanvasCoords],
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
    (e: React.MouseEvent<HTMLElement>) => {
      const coords = clientToCanvasCoords(e.clientX, e.clientY);
      if (!coords) return;
      const stage = semanticRef.current.progress.stage;
      handlePet(
        animRef.current,
        coords.x,
        coords.y,
        emit,
        stage,
        semanticRef.current,
      );
    },
    [clientToCanvasCoords, emit],
  );

  const scaleK = displaySize / CANVAS_SIZE;
  const [spriteHitW, spriteHitH] = STAGE_SIZES[state.progress.stage] ?? [
    28, 18,
  ];
  const hitDiameter = Math.round((spriteHitW + 12) * 1.8 * scaleK);
  const hitHeight = Math.round(
    Math.max(hitDiameter * 0.84, (spriteHitH + 14) * 1.8 * scaleK),
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
        pointerEvents: spritePointer ? "none" : undefined,
        ...style,
      }}
    >
      <canvas
        ref={canvasRef}
        width={displaySize}
        height={displaySize}
        style={{
          width: displaySize,
          height: displaySize,
          display: "block",
          cursor: spritePointer ? "default" : "pointer",
          pointerEvents: spritePointer ? "none" : undefined,
        }}
        onMouseMove={spritePointer ? undefined : onMouseMove}
        onMouseLeave={spritePointer ? undefined : onMouseLeave}
        onMouseDown={spritePointer ? undefined : onMouseDown}
        onMouseUp={spritePointer ? undefined : onMouseUp}
        onClick={spritePointer ? undefined : onClick}
      />
      {spritePointer && (
        <div
          data-testid="buddy-sprite-hit"
          style={{
            position: "absolute",
            left: `calc(50% + ${bubbleView.walkOffsetPx}px)`,
            top: Math.round(CANVAS_CENTER_Y * scaleK),
            width: hitDiameter,
            height: hitHeight,
            transform: "translate(-50%, -50%)",
            borderRadius: 9999,
            pointerEvents: "auto",
            cursor: "pointer",
          }}
          onMouseMove={onMouseMove}
          onMouseLeave={onMouseLeave}
          onMouseDown={onMouseDown}
          onMouseUp={onMouseUp}
          onClick={onClick}
        />
      )}
      {displaySize >= 100 && (
        <BuddySpeechBubble
          text={bubbleView.text}
          textKey={bubbleView.textKey}
          position={bubbleView.position}
          palette={palette}
          visible={bubbleView.visible}
          opacity={bubbleView.opacity}
          compact={compactBubbleOverride || displaySize <= 180}
          width={bubbleView.width}
          maxWidth={bubbleView.maxWidth}
          whiteSpace={bubbleView.whiteSpace}
          anchorStyle={bubbleAnchorStyle(
            bubbleView,
            displaySize,
            state.progress.stage,
            chatCompanionBubbleOverride && displaySize <= 180,
          )}
          intent={speechIntent}
          controls={speechControls}
          onControlClick={onSpeechControlClick}
        />
      )}
    </div>
  );
};
