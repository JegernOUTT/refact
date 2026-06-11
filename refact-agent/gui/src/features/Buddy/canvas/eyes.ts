import {
  fillCircle,
  fillEllipse,
  fillPixel,
  fillRect,
  strokeArc,
  strokeSeg,
} from "./helpers";
import type { BuddyAnimState, ColorMap } from "../types";

function eyeBall(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  size: number,
  color: string,
  grow = 0.55,
): void {
  fillCircle(ctx, x + size / 2, y + size / 2, size / 2 + grow, color);
}

function eyeLidPair(
  ctx: CanvasRenderingContext2D,
  leftX: number,
  leftY: number,
  rightX: number,
  rightY: number,
  size: number,
  lid: number,
  m: ColorMap,
): void {
  if (lid <= 0.28) return;
  const cover = Math.min(size - 0.4, lid * (size + 0.7));
  const drawLid = (x: number, y: number): void => {
    ctx.save();
    ctx.beginPath();
    ctx.arc(x + size / 2, y + size / 2, size / 2 + 0.45, 0, Math.PI * 2);
    ctx.clip();
    fillRect(ctx, x - 0.8, y - 0.8, size + 1.6, cover + 0.8, m.body);
    strokeSeg(ctx, x - 0.3, y + cover, x + size + 0.3, y + cover, m.dark, 0.55);
    ctx.restore();
  };
  drawLid(leftX, leftY);
  drawLid(rightX, rightY);
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
  const lookOffsetX = anim.eyeLookX * 0.8;
  const lookOffsetY = anim.eyeLookY * 0.5;
  const lid = Math.max(anim.lidClose, anim.lidBase);
  const half = size / 2;

  if (lid > 0.78) {
    strokeArc(
      ctx,
      leftX + half,
      leftY + half - 0.4,
      half + 0.2,
      Math.PI * 0.15,
      Math.PI * 0.85,
      m.eyeDark,
      0.7,
    );
    strokeArc(
      ctx,
      rightX + half,
      rightY + half - 0.4,
      half + 0.2,
      Math.PI * 0.15,
      Math.PI * 0.85,
      m.eyeDark,
      0.7,
    );
    return;
  }

  const drawLids = (): void => {
    eyeLidPair(ctx, leftX, leftY, rightX, rightY, size, lid, m);
  };

  if (anim.idleAction === "doze" || anim.moodType === "sleepy") {
    strokeArc(
      ctx,
      leftX + half,
      leftY + 0.6,
      half + 0.3,
      Math.PI * 0.18,
      Math.PI * 0.82,
      m.eyeDark,
      0.7,
    );
    strokeArc(
      ctx,
      rightX + half,
      rightY + 0.6,
      half + 0.3,
      Math.PI * 0.18,
      Math.PI * 0.82,
      m.eyeDark,
      0.7,
    );
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
    eyeBall(ctx, leftX, leftY, size, m.white);
    eyeBall(ctx, rightX, rightY, size, m.white);
    fillCircle(
      ctx,
      Math.max(
        leftX + 0.6,
        Math.min(leftX + size - 0.6, leftX + half + lookOffsetX),
      ),
      Math.max(
        leftY + 0.6,
        Math.min(leftY + size - 0.6, leftY + half + lookOffsetY),
      ),
      0.66,
      m.black,
    );
    fillCircle(
      ctx,
      rightX + half + lookOffsetX,
      rightY + half + lookOffsetY,
      0.66,
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
    fillEllipse(
      ctx,
      leftX + half,
      leftY + half + 0.6,
      half + 0.3,
      half - 0.1,
      m.white,
    );
    fillEllipse(
      ctx,
      rightX + half,
      rightY + half + 0.6,
      half + 0.3,
      half - 0.1,
      m.white,
    );
    fillCircle(
      ctx,
      leftX + half + lookOffsetX,
      leftY + half + 0.7 + lookOffsetY,
      0.66,
      m.black,
    );
    fillCircle(
      ctx,
      rightX + half + lookOffsetX,
      rightY + half + 0.7 + lookOffsetY,
      0.66,
      m.black,
    );
    strokeSeg(
      ctx,
      leftX - 0.4,
      leftY - 0.2,
      leftX + size + 0.4,
      leftY - 1.2,
      "#FF4444",
      0.8,
    );
    strokeSeg(
      ctx,
      rightX - 0.4,
      rightY - 1.2,
      rightX + size + 0.4,
      rightY - 0.2,
      "#FF4444",
      0.8,
    );
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
    eyeBall(ctx, leftX, leftY, size, m.white, 0.85);
    eyeBall(ctx, rightX, rightY, size, m.white, 0.85);
    fillCircle(ctx, leftX + half, leftY + half, 0.7, m.black);
    fillCircle(ctx, rightX + half, rightY + half, 0.7, m.black);
    return;
  }

  if (style === "wink") {
    eyeBall(ctx, leftX, leftY, size, m.white);
    fillCircle(
      ctx,
      Math.max(
        leftX + 0.6,
        Math.min(leftX + size - 0.6, leftX + half + lookOffsetX),
      ),
      Math.max(
        leftY + 0.6,
        Math.min(leftY + size - 0.6, leftY + half + lookOffsetY),
      ),
      0.66,
      m.black,
    );
    strokeArc(
      ctx,
      rightX + half,
      rightY + size - 0.8,
      half + 0.2,
      Math.PI * 1.15,
      Math.PI * 1.85,
      m.eyeDark,
      0.7,
    );
    ctx.globalAlpha = 0.4;
    fillEllipse(ctx, rightX + half, rightY + size + 0.8, 1.7, 0.8, m.rosy);
    ctx.globalAlpha = 1;
    return;
  }

  if (style === "shifty") {
    fillEllipse(
      ctx,
      leftX + half,
      leftY + half + 0.5,
      half + 0.3,
      half - 0.05,
      m.white,
    );
    fillEllipse(
      ctx,
      rightX + half,
      rightY + half + 0.5,
      half + 0.3,
      half - 0.05,
      m.white,
    );
    strokeSeg(
      ctx,
      leftX - 0.2,
      leftY + 0.2,
      leftX + size + 0.2,
      leftY + 0.2,
      m.eyeDark,
      0.7,
    );
    strokeSeg(
      ctx,
      rightX - 0.2,
      rightY + 0.2,
      rightX + size + 0.2,
      rightY + 0.2,
      m.eyeDark,
      0.7,
    );
    const sideX = Math.floor(frame / 20) % 2 === 0 ? 0.6 : size - 0.6;
    fillCircle(ctx, leftX + sideX, leftY + half + 0.4, 0.62, m.black);
    fillCircle(ctx, rightX + sideX, rightY + half + 0.4, 0.62, m.black);
    return;
  }

  eyeBall(ctx, leftX, leftY, size, m.white);
  eyeBall(ctx, rightX, rightY, size, m.white);
  const dilated = anim.pupilDilation > 0.75 && size >= 3;
  const clampPupil = (x: number, y: number, ex: number, ey: number) => ({
    px: Math.max(ex + 0.7, Math.min(ex + size - 0.7, x)),
    py: Math.max(ey + 0.7, Math.min(ey + size - 0.7, y)),
  });
  if (dilated) {
    const lp = clampPupil(
      leftX + half + lookOffsetX,
      leftY + half + lookOffsetY,
      leftX,
      leftY,
    );
    const rp = clampPupil(
      rightX + half + lookOffsetX,
      rightY + half + lookOffsetY,
      rightX,
      rightY,
    );
    fillCircle(ctx, lp.px, lp.py, 1.05, m.black);
    fillCircle(ctx, rp.px, rp.py, 1.05, m.black);
    ctx.globalAlpha = 0.85;
    fillCircle(ctx, lp.px - 0.35, lp.py - 0.35, 0.36, m.white);
    fillCircle(ctx, rp.px - 0.35, rp.py - 0.35, 0.36, m.white);
    ctx.globalAlpha = 1;
  } else {
    const lp = clampPupil(
      leftX + half + lookOffsetX,
      leftY + half + lookOffsetY,
      leftX,
      leftY,
    );
    const rp = clampPupil(
      rightX + half + lookOffsetX,
      rightY + half + lookOffsetY,
      rightX,
      rightY,
    );
    fillCircle(ctx, lp.px, lp.py, 0.68, m.black);
    fillCircle(ctx, rp.px, rp.py, 0.68, m.black);
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

  if (anim.cheekPuffTimer > 0) {
    fillRect(ctx, mx + 1, my + 1, width - 2, 1, m.eyeDark);
    fillPixel(ctx, mx, my, 1, 1, m.eyeDark);
    fillPixel(ctx, mx + width - 1, my, 1, 1, m.eyeDark);
    return;
  }

  if (style === "wide") {
    fillPixel(ctx, mx + 1, my - 1, width - 2, 1, m.eyeDark);
    fillPixel(ctx, mx, my, 1, 2, m.eyeDark);
    fillPixel(ctx, mx + width - 1, my, 1, 2, m.eyeDark);
    fillPixel(ctx, mx + 1, my + 2, width - 2, 1, m.eyeDark);
    fillPixel(ctx, mx + 1, my, width - 2, 2, m.black);
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
    if (mood === "celebrate" && anim.successStreak >= 2) {
      fillEllipse(
        ctx,
        mx + width / 2,
        my + 1.4,
        width / 2 + 0.6,
        1.7,
        m.eyeDark,
      );
      fillEllipse(ctx, mx + width / 2, my + 0.7, width / 2 - 0.4, 0.6, m.white);
      fillEllipse(ctx, mx + width / 2, my + 2.2, width / 2 - 0.8, 0.7, m.rosy);
      return;
    }
    strokeArc(
      ctx,
      mx + width / 2,
      my - 0.4,
      width / 2 + 0.6,
      Math.PI * 0.14,
      Math.PI * 0.86,
      m.eyeDark,
      0.75,
    );
    return;
  }

  if (style === "X" || style === "angry") {
    fillRect(ctx, mx, my, width, 2, m.eyeDark);
    for (let tooth = 0; tooth < width; tooth += 2) {
      fillPixel(ctx, mx + tooth, my, 1, 1, m.white);
    }
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
}
