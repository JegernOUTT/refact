import React, { useEffect, useRef } from "react";
import { useBuddySurfaceVisible } from "./buddyPresenceMotion";
import {
  BUDDY_DREAM_HEIGHT,
  BUDDY_DREAM_WIDTH,
  drawBuddyDreamFrame,
  type BuddyDreamKind,
} from "./buddyDreams";

export interface BuddyDreamCanvasProps {
  kind: BuddyDreamKind;
  reducedMotion?: boolean;
  visible?: boolean;
}

export const BuddyDreamCanvas: React.FC<BuddyDreamCanvasProps> = ({
  kind,
  reducedMotion = false,
  visible = true,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const surfaceVisible = useBuddySurfaceVisible(canvasRef);

  useEffect(() => {
    if (!visible || !surfaceVisible) return;
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;

    if (reducedMotion) {
      drawBuddyDreamFrame(ctx, kind, 30);
      return;
    }

    let rafId = 0;
    let mounted = true;
    let startMs: number | null = null;
    const render = (timestampMs: number) => {
      if (!mounted || document.hidden) return;
      startMs ??= timestampMs;
      drawBuddyDreamFrame(ctx, kind, ((timestampMs - startMs) / 1000) * 24);
      rafId = window.requestAnimationFrame(render);
    };
    rafId = window.requestAnimationFrame(render);
    return () => {
      mounted = false;
      window.cancelAnimationFrame(rafId);
    };
  }, [kind, reducedMotion, visible, surfaceVisible]);

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
