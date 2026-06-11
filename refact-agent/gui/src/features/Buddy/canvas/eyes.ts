import { fillPixel, fillRect } from "./helpers";
import type { BuddyAnimState, ColorMap } from "../types";

export function drawEarOverlay(
  ctx: CanvasRenderingContext2D,
  bodyX: number,
  bodyY: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  const twitching = anim.earTwitchTimer > 0;
  if (Math.abs(anim.earAnimProgress) < 0.1 && !twitching) return;
  const perked = anim.earAnimProgress > 0;
  const offset = Math.round(Math.abs(anim.earAnimProgress) * 2);
  const flick = twitching && anim.earTwitchTimer % 4 < 2 ? 1 : 0;
  const leftFlick = anim.earTwitchSide < 0 ? flick : 0;
  const rightFlick = anim.earTwitchSide > 0 ? flick : 0;
  if (perked || twitching) {
    fillPixel(ctx, bodyX, bodyY - offset - leftFlick, 1, 1, m.body);
    fillPixel(ctx, bodyX + 4, bodyY - offset - rightFlick, 1, 1, m.body);
  } else {
    fillPixel(ctx, bodyX, bodyY + offset + leftFlick, 1, 1, m.dark);
    fillPixel(ctx, bodyX + 4, bodyY + offset + rightFlick, 1, 1, m.dark);
  }
}

const BROWLESS_EYE_STYLES = new Set([
  "angry",
  "X",
  "star",
  "heart",
  "spiral",
  "uwu",
  "squint",
]);

export function drawBrows(
  ctx: CanvasRenderingContext2D,
  leftX: number,
  leftY: number,
  rightX: number,
  rightY: number,
  size: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  if (BROWLESS_EYE_STYLES.has(anim.eyeStyle)) return;
  if (anim.idleAction === "doze") return;
  const mood = anim.moodType;
  const lift = anim.eyeStyle === "wide" ? 1 : 0;

  if (mood === "alert" || anim.eyeStyle === "wide") {
    fillPixel(ctx, leftX, leftY - 3 - lift, size, 1, m.eyeDark);
    fillPixel(ctx, rightX, rightY - 3 - lift, size, 1, m.eyeDark);
    return;
  }
  if (mood === "concerned" || anim.eyeStyle === "teary") {
    fillPixel(ctx, leftX, leftY - 2, 2, 1, m.eyeDark);
    fillPixel(ctx, leftX + 2, leftY - 3, 1, 1, m.eyeDark);
    fillPixel(ctx, rightX + size - 2, rightY - 2, 2, 1, m.eyeDark);
    fillPixel(ctx, rightX, rightY - 3, 1, 1, m.eyeDark);
    return;
  }
  if (mood === "working" || mood === "focused" || mood === "thinking") {
    fillPixel(ctx, leftX, leftY - 2, size, 1, m.eyeDark);
    fillPixel(ctx, rightX, rightY - 2, size, 1, m.eyeDark);
    return;
  }
  if (mood === "curious" || mood === "learning") {
    fillPixel(ctx, rightX, rightY - 3, size, 1, m.eyeDark);
  }
}

export function drawEyes(
  ctx: CanvasRenderingContext2D,
  leftX: number,
  leftY: number,
  rightX: number,
  rightY: number,
  m: ColorMap,
  size: number,
  anim: BuddyAnimState,
): void {
  const lookOffsetX = Math.round(anim.eyeLookX * 0.8);
  const lookOffsetY = Math.round(anim.eyeLookY * 0.5);
  const lid = Math.max(anim.lidClose, anim.lidBase);

  if (lid > 0.78) {
    fillRect(ctx, leftX, leftY + ((size / 2) | 0), size, 1, m.eyeDark);
    fillRect(ctx, rightX, rightY + ((size / 2) | 0), size, 1, m.eyeDark);
    return;
  }

  const drawLids = (): void => {
    if (lid <= 0.28) return;
    const cover = Math.max(1, Math.min(size - 1, Math.round(lid * size)));
    fillRect(ctx, leftX, leftY, size, cover, m.body);
    fillRect(ctx, rightX, rightY, size, cover, m.body);
    fillRect(ctx, leftX, leftY + cover - 1, size, 1, m.dark);
    fillRect(ctx, rightX, rightY + cover - 1, size, 1, m.dark);
  };

  if (anim.idleAction === "doze" || anim.moodType === "sleepy") {
    for (let i = 0; i < size; i++) {
      fillPixel(ctx, leftX + i, leftY + 1, 1, 1, m.eyeDark);
      fillPixel(ctx, rightX + i, rightY + 1, 1, 1, m.eyeDark);
    }
    fillPixel(ctx, leftX - 1, leftY, 1, 1, m.eyeDark);
    fillPixel(ctx, leftX + size, leftY, 1, 1, m.eyeDark);
    fillPixel(ctx, rightX - 1, rightY, 1, 1, m.eyeDark);
    fillPixel(ctx, rightX + size, rightY, 1, 1, m.eyeDark);
    return;
  }

  const style = anim.eyeStyle;
  const frame = anim.frame;

  if (style === "star") {
    const drawStar = (x: number, y: number) => {
      fillPixel(ctx, x + 1, y, 1, 1, m.gold);
      fillPixel(ctx, x, y + 1, 1, 1, m.gold);
      fillPixel(ctx, x + 1, y + 1, 1, 1, m.gold);
      fillPixel(ctx, x + 2, y + 1, 1, 1, m.gold);
      fillPixel(ctx, x + 1, y + 2, 1, 1, m.gold);
      if (size >= 3) {
        fillPixel(ctx, x, y, 1, 1, m.gold);
        fillPixel(ctx, x + 2, y, 1, 1, m.gold);
        fillPixel(ctx, x, y + 2, 1, 1, m.gold);
        fillPixel(ctx, x + 2, y + 2, 1, 1, m.gold);
      }
      if (Math.sin(frame * 0.12 + x) > 0.6)
        fillPixel(ctx, x - 1, y - 1, 1, 1, m.white);
    };
    drawStar(leftX, leftY);
    drawStar(rightX, rightY);
    return;
  }

  if (style === "heart") {
    const drawHeart = (x: number, y: number) => {
      fillPixel(ctx, x, y, 1, 1, m.rosy);
      fillPixel(ctx, x + 2, y, 1, 1, m.rosy);
      fillPixel(ctx, x, y + 1, 1, 1, m.rosy);
      fillPixel(ctx, x + 1, y + 1, 1, 1, m.rosy);
      fillPixel(ctx, x + 2, y + 1, 1, 1, m.rosy);
      fillPixel(ctx, x + 1, y + 2, 1, 1, m.rosy);
      ctx.globalAlpha = 0.4 + Math.sin(frame * 0.1) * 0.3;
      fillPixel(ctx, x - 1, y + 1, 1, 1, m.rosy);
      ctx.globalAlpha = 1;
    };
    drawHeart(leftX, leftY);
    drawHeart(rightX, rightY);
    ctx.globalAlpha = 0.35;
    fillRect(ctx, leftX - 2, leftY + size, 4, 2, m.rosy);
    fillRect(ctx, rightX - 1, rightY + size, 4, 2, m.rosy);
    ctx.globalAlpha = 1;
    return;
  }

  if (style === "spiral") {
    const t = frame * 0.18;
    const drawSpiral = (cx: number, cy: number) => {
      for (let i = 0; i < 4; i++) {
        const a = t + i * Math.PI * 0.5;
        ctx.globalAlpha = 0.4 + i * 0.15;
        fillPixel(
          ctx,
          cx + Math.round(Math.cos(a) * size * 0.4),
          cy + Math.round(Math.sin(a) * size * 0.4),
          1,
          1,
          m.accent,
        );
      }
      ctx.globalAlpha = 1;
    };
    drawSpiral(leftX + ((size / 2) | 0), leftY + ((size / 2) | 0));
    drawSpiral(rightX + ((size / 2) | 0), rightY + ((size / 2) | 0));
    return;
  }

  if (style === "teary") {
    fillRect(ctx, leftX, leftY, size, size, m.white);
    fillRect(ctx, rightX, rightY, size, size, m.white);
    fillPixel(
      ctx,
      Math.max(leftX, Math.min(leftX + size - 1, leftX + 1 + lookOffsetX)),
      Math.max(leftY, Math.min(leftY + size - 1, leftY + 1 + lookOffsetY)),
      1,
      1,
      m.black,
    );
    fillPixel(
      ctx,
      rightX + 1 + lookOffsetX,
      rightY + 1 + lookOffsetY,
      1,
      1,
      m.black,
    );
    const td = (frame * 0.4 + leftX) % 10;
    ctx.globalAlpha = td < 5 ? td / 5 : 1 - (td - 5) / 5;
    fillPixel(
      ctx,
      leftX + ((size / 2) | 0),
      leftY + size + ((td / 3) | 0),
      1,
      1,
      "#60A5FA",
    );
    fillPixel(
      ctx,
      rightX + ((size / 2) | 0),
      rightY + size + ((td / 3) | 0),
      1,
      1,
      "#60A5FA",
    );
    ctx.globalAlpha = 0.3;
    fillPixel(ctx, leftX, leftY, 1, 1, "#93C5FD");
    fillPixel(ctx, rightX, rightY, 1, 1, "#93C5FD");
    ctx.globalAlpha = 1;
    drawLids();
    return;
  }

  if (style === "angry") {
    fillRect(ctx, leftX, leftY + 1, size, size - 1, m.white);
    fillRect(ctx, rightX, rightY + 1, size, size - 1, m.white);
    fillPixel(
      ctx,
      leftX + 1 + lookOffsetX,
      leftY + 2 + lookOffsetY,
      1,
      1,
      m.black,
    );
    fillPixel(
      ctx,
      rightX + 1 + lookOffsetX,
      rightY + 2 + lookOffsetY,
      1,
      1,
      m.black,
    );
    for (let i = 0; i < size + 1; i++) {
      fillPixel(
        ctx,
        leftX + i,
        leftY - 1 + (i < size / 2 ? 1 : 0),
        1,
        1,
        "#FF4444",
      );
      fillPixel(
        ctx,
        rightX + i,
        rightY - 1 + (i >= size / 2 ? 1 : 0),
        1,
        1,
        "#FF4444",
      );
    }
    return;
  }

  if (style === "X") {
    const xColor = "#FF4444";
    const drawX = (x: number, y: number) => {
      fillPixel(ctx, x, y, 1, 1, xColor);
      fillPixel(ctx, x + size - 1, y, 1, 1, xColor);
      if (size >= 3) {
        fillPixel(ctx, x + 1, y + 1, 1, 1, xColor);
        fillPixel(ctx, x + size - 2, y + 1, 1, 1, xColor);
      }
      fillPixel(ctx, x + ((size / 2) | 0), y + ((size / 2) | 0), 1, 1, xColor);
      if (size >= 3) {
        fillPixel(ctx, x + 1, y + size - 2, 1, 1, xColor);
        fillPixel(ctx, x + size - 2, y + size - 2, 1, 1, xColor);
      }
      fillPixel(ctx, x, y + size - 1, 1, 1, xColor);
      fillPixel(ctx, x + size - 1, y + size - 1, 1, 1, xColor);
      ctx.globalAlpha = Math.abs(Math.sin(frame * 0.15));
      fillRect(ctx, x, y, size, size, "rgba(255,0,0,.15)");
      ctx.globalAlpha = 1;
    };
    drawX(leftX, leftY);
    drawX(rightX, rightY);
    return;
  }

  if (style === "squint") {
    for (let i = 0; i < size; i++) {
      const offset = i === 0 || i === size - 1 ? 1 : 0;
      fillPixel(
        ctx,
        leftX + i,
        leftY + ((size / 2 + offset) | 0),
        1,
        1,
        m.eyeDark,
      );
      fillPixel(
        ctx,
        rightX + i,
        rightY + ((size / 2 + offset) | 0),
        1,
        1,
        m.eyeDark,
      );
    }
    ctx.globalAlpha = 0.4;
    fillRect(ctx, leftX - 2, leftY + size + 1, 4, 2, m.rosy);
    fillRect(ctx, rightX - 1, rightY + size + 1, 4, 2, m.rosy);
    ctx.globalAlpha = 1;
    return;
  }

  if (style === "uwu") {
    for (let i = 0; i < size; i++) {
      const off = Math.round(Math.abs(i - size / 2) * 0.8);
      fillPixel(ctx, leftX + i, leftY + size - 1 - off, 1, 1, m.accent);
      fillPixel(ctx, rightX + i, rightY + size - 1 - off, 1, 1, m.accent);
    }
    ctx.globalAlpha = 0.4;
    fillRect(ctx, leftX - 1, leftY + size, 3, 2, m.rosy);
    fillRect(ctx, rightX - 1, rightY + size, 3, 2, m.rosy);
    ctx.globalAlpha = 1;
    return;
  }

  if (style === "wide") {
    fillRect(ctx, leftX, leftY, size, size, m.white);
    fillRect(ctx, rightX, rightY, size, size, m.white);
    fillPixel(ctx, leftX, leftY - 1, size, 1, m.white);
    fillPixel(ctx, rightX, rightY - 1, size, 1, m.white);
    fillPixel(
      ctx,
      leftX + ((size / 2) | 0),
      leftY + ((size / 2) | 0),
      1,
      1,
      m.black,
    );
    fillPixel(
      ctx,
      rightX + ((size / 2) | 0),
      rightY + ((size / 2) | 0),
      1,
      1,
      m.black,
    );
    return;
  }

  if (style === "wink") {
    fillRect(ctx, leftX, leftY, size, size, m.white);
    fillPixel(
      ctx,
      Math.max(leftX, Math.min(leftX + size - 1, leftX + 1 + lookOffsetX)),
      Math.max(leftY, Math.min(leftY + size - 1, leftY + 1 + lookOffsetY)),
      1,
      1,
      m.black,
    );
    for (let i = 0; i < size; i++) {
      const off = Math.round(Math.abs(i - size / 2) * 0.8);
      fillPixel(ctx, rightX + i, rightY + size - 1 - off, 1, 1, m.eyeDark);
    }
    ctx.globalAlpha = 0.4;
    fillRect(ctx, rightX - 1, rightY + size, 3, 2, m.rosy);
    ctx.globalAlpha = 1;
    return;
  }

  if (style === "shifty") {
    fillRect(ctx, leftX, leftY + 1, size, size - 1, m.white);
    fillRect(ctx, rightX, rightY + 1, size, size - 1, m.white);
    fillRect(ctx, leftX, leftY, size, 1, m.eyeDark);
    fillRect(ctx, rightX, rightY, size, 1, m.eyeDark);
    const sideX = Math.floor(frame / 20) % 2 === 0 ? 0 : size - 1;
    fillPixel(
      ctx,
      leftX + sideX,
      leftY + 1 + ((size / 2) | 0) - 1,
      1,
      1,
      m.black,
    );
    fillPixel(
      ctx,
      rightX + sideX,
      rightY + 1 + ((size / 2) | 0) - 1,
      1,
      1,
      m.black,
    );
    return;
  }

  fillRect(ctx, leftX, leftY, size, size, m.white);
  fillRect(ctx, rightX, rightY, size, size, m.white);
  const dilated = anim.pupilDilation > 0.75 && size >= 3;
  if (dilated) {
    const lpx = Math.max(
      leftX,
      Math.min(leftX + size - 2, leftX + 1 + lookOffsetX),
    );
    const lpy = Math.max(
      leftY,
      Math.min(leftY + size - 2, leftY + 1 + lookOffsetY),
    );
    const rpx = Math.max(
      rightX,
      Math.min(rightX + size - 2, rightX + 1 + lookOffsetX),
    );
    const rpy = Math.max(
      rightY,
      Math.min(rightY + size - 2, rightY + 1 + lookOffsetY),
    );
    fillPixel(ctx, lpx, lpy, 2, 2, m.black);
    fillPixel(ctx, rpx, rpy, 2, 2, m.black);
    ctx.globalAlpha = 0.85;
    fillPixel(ctx, lpx, lpy, 1, 1, m.white);
    fillPixel(ctx, rpx, rpy, 1, 1, m.white);
    ctx.globalAlpha = 1;
  } else {
    fillPixel(
      ctx,
      Math.max(leftX, Math.min(leftX + size - 1, leftX + 1 + lookOffsetX)),
      Math.max(leftY, Math.min(leftY + size - 1, leftY + 1 + lookOffsetY)),
      1,
      1,
      m.black,
    );
    fillPixel(
      ctx,
      Math.max(rightX, Math.min(rightX + size - 1, rightX + 1 + lookOffsetX)),
      Math.max(rightY, Math.min(rightY + size - 1, rightY + 1 + lookOffsetY)),
      1,
      1,
      m.black,
    );
  }
  drawLids();
}

export function drawMouth(
  ctx: CanvasRenderingContext2D,
  mx: number,
  my: number,
  m: ColorMap,
  width: number,
  anim: BuddyAnimState,
): void {
  const mood = anim.moodType;
  const style = anim.eyeStyle;
  const frame = anim.frame;

  if (anim.idleAction === "doze") {
    fillPixel(ctx, mx + 1, my, width - 2, 1, m.eyeDark);
    return;
  }

  if (anim.pantTimer > 0) {
    fillRect(ctx, mx + 1, my, width - 2, 2, m.eyeDark);
    fillPixel(ctx, mx + ((width / 2) | 0), my + 2, 1, 1, m.rosy);
    return;
  }

  if (style === "squint" || style === "uwu") {
    fillPixel(ctx, mx - 1, my, 1, 1, m.eyeDark);
    fillPixel(ctx, mx, my + 1, 1, 1, m.eyeDark);
    fillPixel(ctx, mx + 1, my, 1, 1, m.eyeDark);
    fillPixel(ctx, mx + 2, my + 1, 1, 1, m.eyeDark);
    fillPixel(ctx, mx + 3, my, 1, 1, m.eyeDark);
    return;
  }

  if (mood === "happy" || mood === "celebrate") {
    fillPixel(ctx, mx - 1, my, 1, 1, m.eyeDark);
    fillRect(ctx, mx, my + 1, width, 1, m.eyeDark);
    fillPixel(ctx, mx + width, my, 1, 1, m.eyeDark);
    return;
  }

  if (style === "X" || style === "angry") {
    fillRect(ctx, mx, my + 1, width, 1, m.eyeDark);
    if (Math.floor(frame / 4) % 3 === 0)
      fillPixel(ctx, mx + ((width / 2) | 0), my + 1, 1, 1, m.white);
    return;
  }

  if (style === "teary" || mood === "concerned" || mood === "alert") {
    fillRect(ctx, mx, my + 1, width, 1, m.eyeDark);
    fillPixel(ctx, mx - 1, my + 2, 1, 1, m.eyeDark);
    fillPixel(ctx, mx + width, my + 2, 1, 1, m.eyeDark);
    return;
  }

  // focused/working: slightly open determined mouth
  if (mood === "working" || mood === "focused" || mood === "thinking") {
    fillRect(ctx, mx + 1, my + 1, width - 2, 1, m.eyeDark);
    return;
  }

  // learning/curious: open mouth (slight excitement)
  if (mood === "learning" || mood === "curious") {
    fillPixel(ctx, mx, my, 1, 1, m.eyeDark);
    fillRect(ctx, mx + 1, my + 1, width - 2, 1, m.eyeDark);
    fillPixel(ctx, mx + width - 1, my, 1, 1, m.eyeDark);
    return;
  }

  fillRect(ctx, mx + 1, my, width - 2, 1, m.eyeDark);
}
