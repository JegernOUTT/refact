import React, { useEffect, useRef } from "react";
import {
  BUDDY_DREAM_HEIGHT,
  BUDDY_DREAM_WIDTH,
  drawBuddyDreamFrame,
  type BuddyDreamKind,
} from "./buddyDreams";

export interface BuddyDreamCanvasProps {
  kind: BuddyDreamKind;
  reducedMotion?: boolean;
}

export const BuddyDreamCanvas: React.FC<BuddyDreamCanvasProps> = ({
  kind,
  reducedMotion = false,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;

    if (reducedMotion) {
      drawBuddyDreamFrame(ctx, kind, 30);
      return;
    }

    let rafId = 0;
    let startMs: number | null = null;
    const render = (timestampMs: number) => {
      if (!document.hidden) {
        startMs ??= timestampMs;
        drawBuddyDreamFrame(ctx, kind, ((timestampMs - startMs) / 1000) * 24);
      }
      rafId = window.requestAnimationFrame(render);
    };
    rafId = window.requestAnimationFrame(render);
    return () => window.cancelAnimationFrame(rafId);
  }, [kind, reducedMotion]);

  return (
    <canvas
      ref={canvasRef}
      width={BUDDY_DREAM_WIDTH}
      height={BUDDY_DREAM_HEIGHT}
      data-testid="buddy-dream-canvas"
      data-dream={kind}
      style={{ display: "block", width: "100%", height: "auto" }}
    />
  );
};
