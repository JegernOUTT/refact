import { strokeRect } from "./helpers";
import { spawnSparks } from "./particles";
import type { BuddyAnimState, ColorMap } from "../types";

export function renderWalkingFeet(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  spriteWidth: number,
  spriteHeight: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  if (!anim.walking || Math.abs(anim.walkSpeed) < 0.1) return;
  const leftUp = Math.sin(anim.walkPhase) > 0;
  const stepHeight = Math.abs(Math.sin(anim.walkPhase)) * 2.5;
  ctx.fillStyle = m.dark;
  ctx.fillRect(
    (ox + 4) | 0,
    (oy + spriteHeight + (leftUp ? -stepHeight : 0)) | 0,
    3,
    2,
  );
  ctx.fillRect(
    (ox + spriteWidth - 7) | 0,
    (oy + spriteHeight + (!leftUp ? -stepHeight : 0)) | 0,
    3,
    2,
  );
}

export function renderToy(
  ctx: CanvasRenderingContext2D,
  ox: number,
  oy: number,
  spriteWidth: number,
  spriteHeight: number,
  m: ColorMap,
  anim: BuddyAnimState,
): void {
  if (!anim.toyActive || !anim.toyType) return;

  const bounce = Math.abs(Math.sin(anim.toyAnimPhase)) * 3;
  const tx = ox + spriteWidth + 5;
  const ty = (oy + spriteHeight / 2 - 4 - bounce) | 0;

  if (anim.toyType === "duck") {
    ctx.fillStyle = "#FFD700";
    ctx.fillRect(tx, ty, 7, 5);
    ctx.fillRect(tx + 2, ty - 3, 4, 3);
    ctx.fillStyle = "#FF8C00";
    ctx.fillRect(tx - 1, ty + 1, 2, 1);
    ctx.fillStyle = "#000";
    ctx.fillRect(tx + 3, ty - 3, 1, 1);
    ctx.fillStyle = "#FFB800";
    ctx.fillRect(tx, ty + 1, 3, 2);
    if (anim.toyAnimPhase > 3.1 && anim.toyAnimPhase < 3.8) {
      ctx.globalAlpha = 0.6 + Math.sin(anim.toyAnimPhase * 10) * 0.3;
      ctx.fillStyle = "rgba(255,255,255,0.5)";
      ctx.fillRect(tx - 2, ty - 4, 11, 10);
      ctx.globalAlpha = 1;
      if (anim.toyAnimPhase > 3.4 && anim.toyAnimPhase < 3.5)
        spawnSparks(anim, 3, "#FFD700");
    }
  }

  if (anim.toyType === "dice") {
    const binaryValue = Math.floor(anim.toyAnimPhase * 0.7) % 2;
    const spin = Math.abs(Math.sin(anim.toyAnimPhase * 0.5));
    ctx.fillStyle = "#FAFAFA";
    ctx.fillRect(tx, ty, 10, 10);
    strokeRect(ctx, tx, ty, 10, 10, "#334");
    ctx.fillStyle = binaryValue ? "#22C55E" : "#3B82F6";
    ctx.fillRect(tx + 2, ty + 2, 6, 6);
    ctx.fillStyle = "#FFF";
    if (binaryValue) {
      ctx.fillRect(tx + 4, ty + 4, 2, 2);
    } else {
      ctx.fillRect(tx + 2, ty + 2, 2, 2);
      ctx.fillRect(tx + 6, ty + 6, 2, 2);
    }
    ctx.globalAlpha = spin * 0.3;
    ctx.fillStyle = "#FFF";
    ctx.fillRect(tx, ty, 10, 2);
    ctx.globalAlpha = 1;
  }

  if (anim.toyType === "coffee") {
    ctx.fillStyle = "#5C3317";
    ctx.fillRect(tx, ty + 2, 9, 8);
    ctx.fillRect(tx + 9, ty + 3, 2, 5);
    ctx.fillStyle = "#2D0F00";
    ctx.fillRect(tx + 1, ty + 3, 7, 6);
    ctx.fillStyle = "#6B4226";
    const liquidLevel = Math.abs(Math.sin(anim.toyAnimPhase * 0.8));
    ctx.fillRect(tx + 1, (ty + 3 + liquidLevel * 2) | 0, 7, 1);
    for (let i = 0; i < 3; i++) {
      const steamY = ty - 3 - (anim.toyAnimPhase % 4) * 0.5;
      ctx.globalAlpha = 0.35 + Math.sin(anim.toyAnimPhase * 2 + i) * 0.2;
      ctx.fillStyle = "#DDD";
      ctx.fillRect((tx + 2 + i * 2) | 0, steamY | 0, 1, 2);
    }
    ctx.globalAlpha = 1;
  }

  if (anim.toyType === "bug") {
    const bugX = (tx + Math.sin(anim.toyAnimPhase * 2.5) * 7) | 0;
    const bugY = (ty + Math.cos(anim.toyAnimPhase * 1.8) * 3) | 0;
    ctx.fillStyle = "#1A7A1A";
    ctx.fillRect(bugX, bugY, 6, 4);
    ctx.fillStyle = "#FF3333";
    ctx.fillRect(bugX + 1, bugY, 1, 1);
    ctx.fillRect(bugX + 4, bugY, 1, 1);
    ctx.fillStyle = "#1A7A1A";
    const legPhase = anim.toyAnimPhase * 4;
    ctx.fillRect(bugX - 1, bugY + 1, 2, Math.sin(legPhase) > 0 ? 2 : 1);
    ctx.fillRect(bugX + 5, bugY + 1, 2, Math.sin(legPhase) < 0 ? 2 : 1);
    ctx.fillRect(bugX, bugY + 3, 2, 1);
    if (anim.toyDurationTimer < 30) {
      ctx.globalAlpha = anim.toyDurationTimer / 30;
      ctx.fillStyle = "#FFD700";
      ctx.fillRect(bugX - 2, bugY - 2, 10, 8);
      ctx.globalAlpha = 1;
    }
  }

  if (anim.toyType === "scroll") {
    const unrolled = Math.min((anim.toyAnimPhase * 2.5) | 0, 15);
    ctx.fillStyle = "#F5DEB3";
    ctx.fillRect(tx, ty, unrolled, 11);
    ctx.fillStyle = "#445";
    for (let i = 0; i < 3; i++) {
      const lineWidth = Math.max(0, unrolled - 2);
      if (lineWidth > 0)
        ctx.fillRect(tx + 1, ty + 2 + i * 3, Math.min(lineWidth, 4 + i * 2), 1);
    }
    ctx.fillStyle = "#C8A06E";
    ctx.fillRect(tx - 1, ty, 2, 11);
    ctx.fillRect(tx + unrolled - 1, ty, 2, 11);
    if (unrolled > 8) {
      ctx.globalAlpha = 0.15 + Math.sin(anim.toyAnimPhase) * 0.1;
      ctx.fillStyle = "#FFFBAF";
      ctx.fillRect(tx, ty, unrolled, 11);
      ctx.globalAlpha = 1;
    }
  }

  const armX = ox + spriteWidth;
  const armY = (oy + spriteHeight / 2) | 0;
  const armLength = Math.round(
    3 + Math.abs(Math.sin(anim.toyAnimPhase * 0.6)) * 2,
  );
  ctx.fillStyle = m.body;
  ctx.fillRect(armX, armY, armLength, 2);
}
