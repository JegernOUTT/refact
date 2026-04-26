import { fillPixel, fillRect, fillText } from "./helpers";
import type {
  BuddyAnimState,
  Spark,
  FloatingEmoji,
  SleepParticle,
  OrbitingOrb,
  Afterimage,
  SpeedLine,
  GroundFX,
} from "../types";
import { CANVAS_CENTER_X, CANVAS_CENTER_Y } from "../constants";

export function spawnSparks(
  anim: BuddyAnimState,
  count: number,
  color?: string,
): void {
  const cx = CANVAS_CENTER_X + anim.walkOffsetX;
  for (let i = 0; i < count; i++) {
    anim.sparks.push({
      x: cx + (Math.random() - 0.5) * 40,
      y: CANVAS_CENTER_Y + (Math.random() - 0.5) * 20 - 8,
      velocityX: (Math.random() - 0.5) * 1.2,
      velocityY: -0.4 - Math.random() * 1.2,
      life: 1,
      color: color ?? `hsl(${Math.random() * 360},80%,65%)`,
    });
  }
}

export function spawnRainbowSparks(anim: BuddyAnimState, count: number): void {
  const cx = CANVAS_CENTER_X + anim.walkOffsetX;
  for (let i = 0; i < count; i++) {
    anim.sparks.push({
      x: cx + (Math.random() - 0.5) * 50,
      y: CANVAS_CENTER_Y + (Math.random() - 0.5) * 30,
      velocityX: (Math.random() - 0.5) * 2,
      velocityY: -1 - Math.random() * 2,
      life: 1,
      color: `hsl(${(i / count) * 360},100%,60%)`,
    });
  }
}

export function spawnFloatingEmoji(
  anim: BuddyAnimState,
  emoji: string,
  x?: number,
  y?: number,
  count = 1,
): void {
  const cx = CANVAS_CENTER_X + anim.walkOffsetX;
  for (let i = 0; i < count; i++) {
    anim.floatingEmojis.push({
      emoji,
      x: x ?? cx + (Math.random() - 0.5) * 16,
      y: y ?? CANVAS_CENTER_Y - 20 - Math.random() * 8,
      velocityX: (Math.random() - 0.5) * 0.3,
      velocityY: -0.3 - Math.random() * 0.4,
      life: 1,
    });
  }
}

export function spawnSleepParticle(anim: BuddyAnimState): void {
  anim.sleepParticles.push({
    x: CANVAS_CENTER_X + anim.walkOffsetX + 14,
    y: CANVAS_CENTER_Y - 12,
    velocityY: -0.25,
    velocityX: 0.08,
    life: 1,
  });
}

export function spawnOrbitingOrb(
  anim: BuddyAnimState,
  emoji: string,
  count: number,
): void {
  for (let i = 0; i < count; i++) {
    anim.orbitingOrbs.push({
      emoji,
      angle: (i / count) * Math.PI * 2,
      radius: 20 + Math.random() * 6,
      speed: 0.025 + Math.random() * 0.015,
      life: 1,
    });
  }
}

export function spawnAfterimage(anim: BuddyAnimState): void {
  anim.afterimages.push({
    x: CANVAS_CENTER_X + anim.walkOffsetX,
    y: CANVAS_CENTER_Y,
    alpha: 0.6,
    life: 30,
  });
}

export function spawnSpeedLines(
  anim: BuddyAnimState,
  count: number,
  baseAngle: number,
  velocityY = 0,
): void {
  const cx = CANVAS_CENTER_X + anim.walkOffsetX;
  for (let i = 0; i < count; i++) {
    const a = baseAngle + (Math.random() - 0.5) * 1.2;
    anim.speedLines.push({
      x: cx + (Math.random() - 0.5) * 20,
      y: CANVAS_CENTER_Y + (Math.random() - 0.5) * 16,
      angle: a,
      length: 8 + Math.random() * 12,
      life: 1,
      velocityX: Math.cos(a) * 1.5,
      velocityY: velocityY + Math.sin(a) * 1.5,
    });
  }
}

export function spawnGroundEffect(
  anim: BuddyAnimState,
  type: GroundFX["type"],
  x: number,
  y: number,
): void {
  anim.groundFX.push({ x, y, type, life: 1, frame: 0 });
}

export function updateAndRenderSparks(
  ctx: CanvasRenderingContext2D,
  anim: BuddyAnimState,
): void {
  anim.sparks = anim.sparks.filter((s: Spark) => {
    s.x += s.velocityX;
    s.y += s.velocityY;
    s.velocityY += 0.025;
    s.life -= 0.012;
    if (s.life <= 0) return false;
    ctx.globalAlpha = s.life;
    fillPixel(ctx, s.x | 0, s.y | 0, 1, 1, s.color);
    ctx.globalAlpha = 1;
    return true;
  });
}

export function updateAndRenderFloatingEmojis(
  ctx: CanvasRenderingContext2D,
  anim: BuddyAnimState,
): void {
  anim.floatingEmojis = anim.floatingEmojis.filter((f: FloatingEmoji) => {
    f.x += f.velocityX;
    f.y += f.velocityY;
    f.life -= 0.007;
    if (f.life <= 0) return false;
    ctx.globalAlpha = Math.min(1, f.life * 2.5);
    ctx.font = "8px serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(f.emoji, f.x, f.y);
    ctx.globalAlpha = 1;
    return true;
  });
}

export function updateAndRenderSleepParticles(
  ctx: CanvasRenderingContext2D,
  anim: BuddyAnimState,
  accentColor: string,
  frame: number,
): void {
  if (anim.idleAction === "doze" && frame % 30 === 0) spawnSleepParticle(anim);
  anim.sleepParticles = anim.sleepParticles.filter((z: SleepParticle) => {
    z.y += z.velocityY;
    z.x += z.velocityX + Math.sin(frame * 0.05 + z.y * 0.08) * 0.15;
    z.life -= 0.005;
    if (z.life <= 0) return false;
    ctx.globalAlpha = z.life * 0.7;
    fillText(ctx, "z", z.x, z.y, 6, accentColor);
    ctx.globalAlpha = 1;
    return true;
  });
}

export function updateAndRenderOrbitingOrbs(
  ctx: CanvasRenderingContext2D,
  anim: BuddyAnimState,
): void {
  const cx = CANVAS_CENTER_X + anim.walkOffsetX;
  anim.orbitingOrbs = anim.orbitingOrbs.filter((o: OrbitingOrb) => {
    o.angle += o.speed;
    o.life -= 0.004;
    if (o.life <= 0) return false;
    const x = cx + Math.cos(o.angle) * o.radius;
    const y = CANVAS_CENTER_Y - 2 + Math.sin(o.angle) * o.radius * 0.4;
    ctx.globalAlpha = Math.min(1, o.life * 3);
    ctx.font = "7px serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(o.emoji, x, y);
    ctx.globalAlpha = 1;
    return true;
  });
}

export function updateAndRenderAfterimages(
  ctx: CanvasRenderingContext2D,
  anim: BuddyAnimState,
  bodyColor: string,
): void {
  anim.afterimages = anim.afterimages.filter((a: Afterimage) => {
    a.life--;
    a.alpha *= 0.9;
    if (a.life <= 0 || a.alpha < 0.02) return false;
    ctx.globalAlpha = a.alpha * 0.3;
    fillRect(
      ctx,
      a.x + anim.nuzzleOffsetX - 10,
      a.y + anim.nuzzleOffsetY - 8,
      20,
      16,
      bodyColor,
    );
    ctx.globalAlpha = 1;
    return true;
  });
}

export function updateAndRenderSpeedLines(
  ctx: CanvasRenderingContext2D,
  anim: BuddyAnimState,
): void {
  anim.speedLines = anim.speedLines.filter((l: SpeedLine) => {
    l.x += l.velocityX;
    l.y += l.velocityY;
    l.life -= 0.025;
    if (l.life <= 0) return false;
    ctx.globalAlpha = l.life * 0.6;
    ctx.strokeStyle = "#FFF";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(l.x | 0, l.y | 0);
    ctx.lineTo(
      (l.x + Math.cos(l.angle) * l.length) | 0,
      (l.y + Math.sin(l.angle) * l.length) | 0,
    );
    ctx.stroke();
    ctx.globalAlpha = 1;
    return true;
  });
}

export function updateAndRenderGroundEffects(
  ctx: CanvasRenderingContext2D,
  anim: BuddyAnimState,
  accentColor: string,
): void {
  anim.groundFX = anim.groundFX.filter((g: GroundFX) => {
    g.life -= 0.015;
    g.frame++;
    if (g.life <= 0) return false;
    ctx.globalAlpha = g.life;
    if (g.type === "impact") {
      const r = Math.round((1 - g.life) * 12);
      ctx.fillStyle = accentColor;
      ctx.fillRect(g.x - r, g.y - r / 3, r * 2, 1);
      ctx.fillRect(g.x - r, g.y - r / 3 + r - 1, r * 2, 1);
      ctx.fillRect(g.x - r, g.y - r / 3, 1, r);
      ctx.fillRect(g.x + r - 1, g.y - r / 3, 1, r);
    } else if ((g.type as string) === "crack") {
      fillPixel(ctx, g.x, g.y, 1, 1, "#666");
      fillPixel(ctx, g.x - 1, g.y + 1, 1, 1, "#555");
      fillPixel(ctx, g.x + 1, g.y + 1, 1, 1, "#555");
      if (g.frame > 3) {
        fillPixel(ctx, g.x - 2, g.y + 2, 1, 1, "#444");
        fillPixel(ctx, g.x + 2, g.y + 2, 1, 1, "#444");
      }
      if (g.frame > 6) {
        fillPixel(ctx, g.x - 3, g.y + 2, 1, 1, "#333");
        fillPixel(ctx, g.x + 3, g.y + 1, 1, 1, "#333");
      }
    } else if ((g.type as string) === "dust") {
      const s = Math.round((1 - g.life) * 4);
      ctx.globalAlpha = g.life * 0.4;
      fillPixel(ctx, g.x - s, g.y - s / 2, 1, 1, "#888");
      fillPixel(ctx, g.x + s, g.y - s / 2, 1, 1, "#888");
      fillPixel(ctx, g.x, g.y - s, 1, 1, "#999");
    }
    ctx.globalAlpha = 1;
    return true;
  });
}
