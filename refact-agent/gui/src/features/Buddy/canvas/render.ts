import { fillRect, fillPixel, strokeArc, strokeEllipse } from "./helpers";
import { buildColorMap } from "./colorMap";
import { drawStageCharacter } from "./sprites";
import { renderToy } from "./toys";
import {
  updateAndRenderSparks,
  updateAndRenderFloatingEmojis,
  updateAndRenderSleepParticles,
  updateAndRenderOrbitingOrbs,
  updateAndRenderAfterimages,
  updateAndRenderSpeedLines,
  updateAndRenderGroundEffects,
} from "./particles";
import {
  CANVAS_SIZE,
  CANVAS_CENTER_X,
  CANVAS_CENTER_Y,
  STAGE_SIZES,
  PALETTES,
} from "../constants";
import type { BuddyAnimState, BuddySemanticState } from "../types";

export function renderFrame(
  ctx: CanvasRenderingContext2D,
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
): void {
  ctx.clearRect(0, 0, CANVAS_SIZE, CANVAS_SIZE);
  ctx.imageSmoothingEnabled = false;

  const stage = Math.max(0, Math.min(6, semantic.progress.stage));
  const pal = PALETTES[semantic.paletteIndex] ?? PALETTES[0];
  const m = buildColorMap(semantic.paletteIndex);
  const [spriteW, spriteH] = STAGE_SIZES[stage] ?? [28, 18];

  updateAndRenderAfterimages(ctx, anim, pal.body);

  // shakeIntensity is decayed in stepAnimFrame; read-only here
  const shakeX =
    anim.shakeIntensity > 0.3
      ? Math.round((Math.random() - 0.5) * anim.shakeIntensity)
      : 0;
  const shakeY =
    anim.shakeIntensity > 0.3
      ? Math.round((Math.random() - 0.5) * anim.shakeIntensity)
      : 0;

  const isSleeping = anim.idleAction === "doze";
  const bobAmount = isSleeping ? 1 : anim.walking ? 1 : 2;

  let extraBob = 0;
  if (anim.walking) {
    extraBob = -Math.abs(Math.sin(anim.walkPhase)) * 2.5;
  } else {
    if (anim.idleAction === "stretch")
      extraBob = Math.sin(anim.idleActionTimer * 0.1) * 2;
    if (anim.idleAction === "tap")
      extraBob = Math.abs(Math.sin(anim.idleActionTimer * 0.3)) * -1;
    if (anim.idleAction === "yawn")
      extraBob = Math.sin(anim.idleActionTimer * 0.05) * 1;
    if (anim.quirkType === "rock")
      extraBob += Math.sin(anim.stageQuirkTick * 0.08) * 2;
  }

  const moodDropY = semantic.mood.happiness < 38 ? 1 : 0;
  const lowEnergyDropY = semantic.mood.energy < 20 ? 2 : 0;
  const confidentLiftY = semantic.personality.confidence > 65 ? -1 : 0;
  const moodPostureY = moodDropY + lowEnergyDropY + confidentLiftY;

  const bobY = Math.round(
    Math.sin(anim.bobPhase) * bobAmount +
      extraBob +
      moodPostureY -
      anim.levitationOffset,
  );
  const celebBounce =
    anim.celebrationTimer > 0
      ? Math.round(Math.abs(Math.sin(anim.frame * 0.18)) * 5)
      : 0;

  // headTilt contributes as body lean when cursor isn't close enough to override
  let leanX = Math.round(anim.headTilt * 2);
  if (anim.mouseProximity > 0.3 && !anim.walking)
    leanX = Math.round(anim.cursorTargetX * anim.mouseProximity * 3);
  if (anim.walking) leanX += anim.walkDirection * 2;
  if (!anim.walking && !isSleeping && anim.mouseProximity <= 0.3)
    leanX += Math.round(Math.sin(anim.frame * 0.011) * 1.5);
  const shiverX = anim.shiverTimer > 0 ? (anim.frame % 2 === 0 ? 1 : -1) : 0;

  const baseOX = Math.round(
    CANVAS_CENTER_X - spriteW / 2 + anim.nuzzleOffsetX + anim.walkOffsetX,
  );
  const baseOY = Math.round(CANVAS_CENTER_Y - spriteH / 2 + anim.nuzzleOffsetY);
  const ox = baseOX + shakeX + leanX + shiverX;
  const oy = baseOY + bobY - celebBounce + shakeY;

  updateAndRenderGroundEffects(ctx, anim, pal.accent);

  ctx.globalAlpha = 0.12;
  const shadowWidth = Math.round((spriteW - 6) * anim.squashX);
  fillRect(
    ctx,
    ox + spriteW / 2 - shadowWidth / 2,
    oy + spriteH + 1,
    shadowWidth,
    2,
    "#000",
  );
  ctx.globalAlpha = 1;

  updateAndRenderSpeedLines(ctx, anim);

  if (anim.heat > 40) {
    const heatAlpha = (anim.heat - 40) / 120;
    ctx.globalAlpha = heatAlpha * 0.35;
    ctx.fillStyle = `hsl(${20 + anim.heat / 2},100%,60%)`;
    for (let i = 0; i < 4; i++) {
      const a = anim.frame * 0.04 + i * 1.57;
      const r = 8 + anim.heat / 20;
      ctx.fillRect(
        ox + spriteW / 2 + Math.cos(a) * r - 2,
        oy + spriteH / 2 + Math.sin(a) * r * 0.5 - 2,
        4,
        4,
      );
    }
    ctx.globalAlpha =
      (heatAlpha * 0.7 + Math.sin(anim.frame * 0.08) * 0.3 * heatAlpha) * 0.15;
    ctx.fillStyle = "#FF6600";
    ctx.fillRect(ox, oy, spriteW, spriteH);
    ctx.globalAlpha = 1;
  }

  ctx.save();
  const scale = 1.8;
  const scx = CANVAS_CENTER_X + anim.walkOffsetX;
  const scy = CANVAS_CENTER_Y;
  ctx.translate(scx, scy);
  ctx.scale(scale, scale);
  ctx.translate(-scx, -scy);
  ctx.save();
  const centerX = ox + spriteW / 2;
  const centerY = oy + spriteH / 2;
  ctx.translate(centerX, centerY);
  // breathScale adds energy-based breathing oscillation on top of squash
  ctx.scale(anim.squashX, anim.squashY + anim.breathScale);
  ctx.translate(-centerX, -centerY);
  drawStageCharacter(ctx, stage, ox, oy, m, anim, semantic.paletteIndex);
  if (anim.blushTimer > 0 || semantic.mood.affection > 70) {
    ctx.globalAlpha = 0.3 + Math.sin(anim.frame * 0.15) * 0.1;
    const cheekY = oy + Math.round(spriteH * 0.62);
    fillRect(ctx, ox + Math.round(spriteW * 0.14), cheekY, 3, 1, "#FB7185");
    fillRect(
      ctx,
      ox + spriteW - Math.round(spriteW * 0.14) - 3,
      cheekY,
      3,
      1,
      "#FB7185",
    );
    ctx.globalAlpha = 1;
  }
  ctx.restore();
  ctx.restore();

  renderToy(ctx, ox, oy, spriteW, spriteH, m, anim);

  if (anim.quirkType === "shell_fall" && anim.quirkActive) {
    const t = anim.stageQuirkTick;
    if (t < 60) {
      ctx.globalAlpha = 1 - t / 60;
      fillRect(ctx, ox + 8, oy + 22 + t / 4, 3, 2, m.belly);
      fillRect(ctx, ox + 8, oy + 23 + t / 4, 3, 1, m.outline);
      ctx.globalAlpha = 1;
    }
  }

  if (anim.idleAction === "curious" && anim.mouseProximity > 0.4) {
    if (Math.sin(anim.frame * 0.2) > 0.3) {
      fillPixel(ctx, ox + spriteW / 2, oy - 6, 1, 3, "#FFF");
      fillPixel(ctx, ox + spriteW / 2, oy - 2, 1, 1, "#FFF");
    }
  }
  if (
    anim.idleAction === "yawn" &&
    anim.idleActionTimer > 20 &&
    anim.idleActionTimer < 50
  ) {
    ctx.globalAlpha = 0.6;
    fillRect(
      ctx,
      ox + spriteW / 2 - 1,
      oy + spriteH / 2 + 4,
      3,
      2,
      pal.eyeDark,
    );
    ctx.globalAlpha = 1;
  }
  if (anim.idleAction === "confidentPose") {
    ctx.globalAlpha = 0.25 + Math.sin(anim.frame * 0.1) * 0.15;
    strokeEllipse(
      ctx,
      ox + spriteW / 2,
      oy + spriteH / 2,
      spriteW / 2 + 3,
      spriteH / 2 + 2,
      pal.accent,
    );
    ctx.globalAlpha = 1;
  }

  if (anim.idleAction === "type_code" && !isSleeping) {
    const keyPhase = Math.floor(anim.frame / 6) % 4;
    const kbX = ox + spriteW / 2 - 5;
    const kbY = oy + spriteH + 2;
    ctx.globalAlpha = 0.5;
    fillRect(ctx, kbX, kbY, 10, 3, pal.dark);
    ctx.globalAlpha = 0.9;
    fillRect(ctx, kbX + keyPhase * 2, kbY, 2, 2, pal.accent);
    ctx.globalAlpha = 1;
  }

  if (anim.idleAction === "spin" && anim.frame % 3 === 0) {
    ctx.globalAlpha = 0.18 + Math.sin(anim.frame * 0.4) * 0.12;
    const spinAngle = anim.frame * 0.12;
    strokeArc(
      ctx,
      ox + spriteW / 2,
      oy + spriteH / 2,
      Math.max(spriteW, spriteH) / 2 + 4,
      spinAngle,
      spinAngle + Math.PI * 1.45,
      pal.accent,
    );
    ctx.globalAlpha = 1;
  }

  if (anim.celebrationTimer > 0 || anim.idleAction === "doze") {
    for (let i = 0; i < 3; i++) {
      const ddx = ox - 3 - i * 3;
      const ddy =
        oy - 2 - i * 4 + Math.round(Math.sin(anim.frame * 0.1 + i) * 1.5);
      ctx.globalAlpha = 0.4 + Math.sin(anim.frame * 0.1 + i * 1.5) * 0.3;
      fillRect(ctx, ddx, ddy, 2, 2, "rgba(255,255,255,.5)");
    }
    ctx.globalAlpha = 1;
  }

  updateAndRenderSparks(ctx, anim);
  updateAndRenderFloatingEmojis(ctx, anim);
  updateAndRenderSleepParticles(ctx, anim, pal.accent, anim.frame);
  updateAndRenderOrbitingOrbs(ctx, anim);

  // Combo sparks are spawned in stepAnimFrame (render.ts is read-only)

  if (anim.screenFlash > 0.01) {
    ctx.globalAlpha = anim.screenFlash;
    fillRect(ctx, 0, 0, CANVAS_SIZE, CANVAS_SIZE, "#FFF");
    ctx.globalAlpha = 1;
  }

  if (anim.screenGlitch > 0.01) {
    ctx.globalAlpha = anim.screenGlitch * 0.3;
    for (let y = 0; y < CANVAS_SIZE; y += 3)
      fillRect(ctx, 0, y, CANVAS_SIZE, 1, "#000");
    if (Math.random() < anim.screenGlitch) {
      const gy = (Math.random() * CANVAS_SIZE) | 0;
      const gh = (2 + Math.random() * 4) | 0;
      const shift = ((Math.random() - 0.5) * 8) | 0;
      try {
        const d = ctx.getImageData(0, gy, CANVAS_SIZE, gh);
        ctx.putImageData(d, shift, gy);
      } catch (_) {
        void 0;
      }
    }
    ctx.globalAlpha = 1;
  }
}
