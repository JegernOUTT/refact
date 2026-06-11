import {
  TAU,
  alphaForMotion,
  clamp01,
  countForMotion,
  drawPixelText,
  drawSpark,
  fillCircle,
  fillEllipse,
  fillPixelRect,
  finiteOr,
  lerp,
  pctX,
  pctY,
  safeDimension,
  safeFrame,
  seededUnit,
  strokeCircle,
  strokeEllipse,
  strokeLine,
  wave,
  type BuddyWorldActor,
  type DrawBuddyWorldBaseArgs,
  type Point,
} from "./buddyWorldDrawHelpers";

interface ActorSpot {
  x: number;
  y: number;
  groundY: number;
}

function easeInOutQuad(progress: number): number {
  const t = clamp01(progress);
  return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
}

function travelProgress(actor: BuddyWorldActor): number {
  if (!actor.travel) return 1;
  const duration = Math.max(1, finiteOr(actor.travel.durationMs, 3800));
  const elapsed =
    finiteOr(actor.nowMs, 0) - finiteOr(actor.travel.startedAtMs, 0);
  return clamp01(elapsed / duration);
}

function actorSpot(
  args: DrawBuddyWorldBaseArgs,
  actor: BuddyWorldActor,
): ActorSpot {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const progress = easeInOutQuad(travelProgress(actor));
  const fromX = finiteOr(actor.travel?.fromXPercent, actor.xPercent);
  const fromY = finiteOr(actor.travel?.fromYPercent, actor.yPercent);
  const xPercent = lerp(fromX, finiteOr(actor.xPercent, 50), progress);
  const yPercent = lerp(fromY, finiteOr(actor.yPercent, 78), progress);
  const x = pctX(width, xPercent);
  const groundY = pctY(height, Math.max(yPercent, 58)) + 14;
  return { x, y: pctY(height, yPercent), groundY };
}

function drawTravelDust(
  args: DrawBuddyWorldBaseArgs,
  actor: BuddyWorldActor,
  spot: ActorSpot,
): void {
  if (args.reducedMotion) return;
  const progress = travelProgress(actor);
  if (progress >= 1) return;
  const frame = safeFrame(args.frame);
  const fromX = pctX(
    safeDimension(args.width, 720),
    finiteOr(actor.travel?.fromXPercent, actor.xPercent),
  );
  const direction = Math.sign(spot.x - fromX) || 1;
  const puffCount = countForMotion(4, args.compact, args.reducedMotion);

  for (let index = 0; index < puffCount; index += 1) {
    const lag = (index + 1) * 7;
    const px = spot.x - direction * lag;
    const fade = 1 - index / puffCount;
    const bounce = Math.abs(Math.sin(frame / 3.6 + index)) * 2.4;
    fillCircle(
      args.ctx,
      px,
      spot.groundY - 2 - bounce * 0.4,
      2.2 - index * 0.35,
      "#D6CDBF",
      alphaForMotion(0.3 * fade, args.reducedMotion),
    );
  }
  if (Math.abs(Math.sin(frame / 3.6)) > 0.82) {
    fillEllipse(
      args.ctx,
      spot.x - direction * 3,
      spot.groundY,
      4.6,
      1.6,
      "#C9BFAE",
      alphaForMotion(0.34, args.reducedMotion),
    );
  }
}

type AccentDrawer = (
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
) => void;

function heldProgress(heldMs: number, totalMs: number): number {
  return clamp01(finiteOr(heldMs, 0) / Math.max(1, finiteOr(totalMs, 1)));
}

function heldStage(heldMs: number, stageMs: number, stages: number): number {
  const safeStage = Math.max(1, finiteOr(stageMs, 1));
  return Math.min(
    Math.max(0, stages - 1),
    Math.floor(Math.max(0, finiteOr(heldMs, 0)) / safeStage),
  );
}

function drawPondAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  const cycle = (frame % 72) / 72;
  for (let ring = 0; ring < 2; ring += 1) {
    const t = clamp01(cycle + ring * 0.42) % 1;
    strokeEllipse(
      args.ctx,
      spot.x - 14,
      spot.groundY + 4,
      6 + t * 16,
      2 + t * 4.4,
      "#7DD3FC",
      1.2,
      alphaForMotion((1 - t) * 0.34, args.reducedMotion),
    );
  }
  drawSpark(
    args.ctx,
    spot.x - 14 + wave(frame, 26, 1, 5, args.reducedMotion),
    spot.groundY - 8,
    1.5,
    "#BAE6FD",
    alphaForMotion(0.5, args.reducedMotion),
  );
}

function drawFireAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  const count = countForMotion(4, args.compact, args.reducedMotion);
  for (let index = 0; index < count; index += 1) {
    const rise = ((frame * 0.8 + index * 21) % 60) / 60;
    const px =
      spot.x + 12 + wave(frame, 14 + index, index * 2, 2.4, args.reducedMotion);
    const py = spot.y - 4 - rise * 22;
    fillPixelRect(
      args.ctx,
      px,
      py,
      1.8,
      1.8,
      index % 2 === 0 ? "#FDBA74" : "#FBBF24",
      alphaForMotion((1 - rise) * 0.66, args.reducedMotion),
    );
  }
  fillCircle(
    args.ctx,
    spot.x + 10,
    spot.y - 2,
    9,
    "#FB923C",
    alphaForMotion(
      0.1 + Math.abs(wave(frame, 18, 0, 0.05, args.reducedMotion)),
      args.reducedMotion,
    ),
  );
}

function drawPuddleAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  const burst = (frame % 34) / 34;
  if (burst < 0.5) {
    const t = burst / 0.5;
    for (let drop = 0; drop < 5; drop += 1) {
      const angle = -Math.PI * (0.22 + drop * 0.14);
      const radius = 4 + t * 11;
      fillPixelRect(
        args.ctx,
        spot.x + Math.cos(angle) * radius,
        spot.groundY - 2 + Math.sin(angle) * radius + t * t * 9,
        1.6,
        1.6,
        "#93C5FD",
        alphaForMotion((1 - t) * 0.7, args.reducedMotion),
      );
    }
  }
  strokeEllipse(
    args.ctx,
    spot.x,
    spot.groundY + 2,
    8 + burst * 6,
    2.4,
    "#BFDBFE",
    1,
    alphaForMotion((1 - burst) * 0.4, args.reducedMotion),
  );
}

function drawSnowAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const burst = (frame % 46) / 46;
  for (let index = 0; index < 6; index += 1) {
    const angle = Math.PI + index * 0.42 + burst * 0.6;
    const radius = 3 + burst * 13;
    fillPixelRect(
      args.ctx,
      spot.x + Math.cos(angle) * radius * 0.9,
      spot.groundY - 3 - Math.sin(burst * Math.PI) * 8 + index,
      1.8,
      1.8,
      "#F8FAFC",
      alphaForMotion((1 - burst) * 0.74, args.reducedMotion),
    );
  }
  fillEllipse(
    args.ctx,
    spot.x + 9,
    spot.groundY + 2,
    7,
    2.6,
    "#E0F2FE",
    alphaForMotion(0.5, args.reducedMotion),
  );

  const stage = heldStage(heldMs, 2_600, 4);
  const bx = spot.x + 19;
  const baseY = spot.groundY + 1;
  if (stage >= 0 && heldMs > 700) {
    const grow = stage === 0 ? heldProgress(heldMs - 700, 1_800) : 1;
    fillCircle(
      args.ctx,
      bx,
      baseY - 4 * grow,
      3 + 3.6 * grow,
      "#F1F5F9",
      alphaForMotion(0.92, args.reducedMotion),
    );
    fillEllipse(
      args.ctx,
      bx,
      baseY + 2,
      7 * grow + 2,
      1.8,
      "#CBDCE8",
      alphaForMotion(0.4, args.reducedMotion),
    );
  }
  if (stage >= 1) {
    fillCircle(
      args.ctx,
      bx,
      baseY - 12,
      4.6,
      "#F1F5F9",
      alphaForMotion(0.94, args.reducedMotion),
    );
  }
  if (stage >= 2) {
    fillCircle(
      args.ctx,
      bx,
      baseY - 19,
      3.2,
      "#F8FAFC",
      alphaForMotion(0.95, args.reducedMotion),
    );
    fillPixelRect(args.ctx, bx - 1.6, baseY - 20, 1.2, 1.2, "#1E293B", 0.9);
    fillPixelRect(args.ctx, bx + 0.6, baseY - 20, 1.2, 1.2, "#1E293B", 0.9);
    fillPixelRect(args.ctx, bx - 0.4, baseY - 18, 1, 1, "#FB923C", 0.92);
  }
  if (stage >= 3) {
    strokeLine(
      args.ctx,
      { x: bx - 4, y: baseY - 13 },
      { x: bx - 9, y: baseY - 17 + wave(frame, 26, 0, 1, args.reducedMotion) },
      "#6B4F3A",
      1.2,
      0.9,
    );
    strokeLine(
      args.ctx,
      { x: bx + 4, y: baseY - 13 },
      { x: bx + 9, y: baseY - 17 },
      "#6B4F3A",
      1.2,
      0.9,
    );
    drawSpark(
      args.ctx,
      bx + 7,
      baseY - 24 + wave(frame, 18, 1, 1.6, args.reducedMotion),
      1.5,
      "#BAE6FD",
      alphaForMotion(0.7, args.reducedMotion),
    );
  }
}

function drawLeafAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const colors = ["#FB923C", "#D97706", "#F87171"];
  const count = countForMotion(5, args.compact, args.reducedMotion);
  for (let index = 0; index < count; index += 1) {
    const swirl = frame / 16 + index * (TAU / count);
    const radius = 8 + seededUnit(401, index) * 7;
    fillPixelRect(
      args.ctx,
      spot.x + Math.cos(swirl) * radius,
      spot.y - 6 + Math.sin(swirl) * radius * 0.5,
      2.4,
      1.8,
      colors[index % colors.length],
      alphaForMotion(0.6, args.reducedMotion),
    );
  }

  const pile = heldProgress(heldMs, 6_500);
  if (pile > 0.1) {
    const px = spot.x + 17;
    fillEllipse(
      args.ctx,
      px,
      spot.groundY + 1,
      4 + pile * 8,
      1.6 + pile * 3,
      "#B45309",
      alphaForMotion(0.84, args.reducedMotion),
    );
    fillEllipse(
      args.ctx,
      px - 2,
      spot.groundY - 1 - pile * 2,
      3 + pile * 5,
      1.4 + pile * 2,
      "#D97706",
      alphaForMotion(0.8, args.reducedMotion),
    );
    fillPixelRect(
      args.ctx,
      px + pile * 4,
      spot.groundY - 2 - pile * 3,
      2.2,
      1.6,
      "#FB923C",
      alphaForMotion(0.85, args.reducedMotion),
    );
  }
  if (pile >= 1) {
    const burst = (frame % 40) / 40;
    for (let index = 0; index < 4; index += 1) {
      const angle = -Math.PI * (0.2 + index * 0.2);
      fillPixelRect(
        args.ctx,
        spot.x + 17 + Math.cos(angle) * (4 + burst * 12),
        spot.groundY - 4 + Math.sin(angle) * (6 + burst * 10) + burst * 8,
        2.2,
        1.6,
        colors[index % colors.length],
        alphaForMotion((1 - burst) * 0.8, args.reducedMotion),
      );
    }
  }
}

function drawFlowerAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  for (let index = 0; index < 4; index += 1) {
    const rise = ((frame * 0.55 + index * 17) % 52) / 52;
    fillPixelRect(
      args.ctx,
      spot.x - 6 + index * 5 + wave(frame, 20, index, 2.4, args.reducedMotion),
      spot.y - 4 - rise * 16,
      1.8,
      1.8,
      index % 2 === 0 ? "#F9A8D4" : "#FBCFE8",
      alphaForMotion((1 - rise) * 0.66, args.reducedMotion),
    );
  }
}

function drawGardenAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const dig = Math.abs(Math.sin(frame / 9));
  const grow = heldProgress(heldMs, 7_200);
  fillEllipse(
    args.ctx,
    spot.x + 8,
    spot.groundY + 1,
    6,
    2,
    "#92400E",
    alphaForMotion(0.5, args.reducedMotion),
  );
  for (let index = 0; index < 3; index += 1) {
    fillPixelRect(
      args.ctx,
      spot.x + 4 + index * 4,
      spot.groundY - 3 - dig * (4 + index * 2),
      1.6,
      1.6,
      "#A16207",
      alphaForMotion(0.6 * dig * (1 - grow * 0.6), args.reducedMotion),
    );
  }
  const drip = (frame % 30) / 30;
  if (grow < 0.92) {
    fillPixelRect(
      args.ctx,
      spot.x + 12 + drip * 3,
      spot.groundY - 12 + drip * 9,
      1.4,
      2,
      "#7DD3FC",
      alphaForMotion((1 - drip) * 0.7, args.reducedMotion),
    );
  }
  const stemH = 3 + grow * 9;
  fillPixelRect(
    args.ctx,
    spot.x + 14,
    spot.groundY - 1 - stemH,
    2,
    stemH,
    "#16A34A",
    alphaForMotion(0.8, args.reducedMotion),
  );
  const sway = wave(frame, 22, 1, 1.2 + grow, args.reducedMotion);
  fillEllipse(
    args.ctx,
    spot.x + 11 + sway,
    spot.groundY - stemH - 1,
    2.2 + grow * 3,
    1.2 + grow * 1.6,
    "#4ADE80",
    alphaForMotion(0.78, args.reducedMotion),
  );
  fillEllipse(
    args.ctx,
    spot.x + 18 + sway,
    spot.groundY - stemH - 2.4,
    2 + grow * 2.6,
    1.1 + grow * 1.4,
    "#86EFAC",
    alphaForMotion(0.74, args.reducedMotion),
  );
  if (grow > 0.78) {
    fillPixelRect(
      args.ctx,
      spot.x + 14.4 + sway,
      spot.groundY - stemH - 4,
      2.2,
      2.2,
      "#F9A8D4",
      alphaForMotion(0.9, args.reducedMotion),
    );
    drawSpark(
      args.ctx,
      spot.x + 20,
      spot.groundY - stemH - 8,
      1.4,
      "#FDE68A",
      alphaForMotion((grow - 0.78) * 3, args.reducedMotion),
    );
  }
}

function drawNapAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  for (let index = 0; index < 3; index += 1) {
    const rise = ((frame * 0.4 + index * 26) % 78) / 78;
    drawPixelText(
      args.ctx,
      "z",
      spot.x + 10 + rise * 8 + index * 2,
      spot.y - 16 - rise * 18,
      "#C7D2FE",
      alphaForMotion((1 - rise) * 0.7, args.reducedMotion),
    );
  }
  const leafFall = ((frame * 0.5) % 90) / 90;
  fillPixelRect(
    args.ctx,
    spot.x - 12 + wave(frame, 17, 1, 5, args.reducedMotion),
    spot.y - 30 + leafFall * 26,
    2.2,
    1.8,
    "#86EFAC",
    alphaForMotion((1 - leafFall) * 0.6, args.reducedMotion),
  );
}

function drawKodamaAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  strokeCircle(
    args.ctx,
    spot.x - 12,
    spot.y - 10,
    6 + Math.abs(wave(frame, 22, 0, 2.4, args.reducedMotion)),
    "#E8EEF4",
    1,
    alphaForMotion(0.34, args.reducedMotion),
  );
  drawSpark(
    args.ctx,
    spot.x - 12,
    spot.y - 10,
    1.4,
    "#F1F5F9",
    alphaForMotion(0.66, args.reducedMotion),
  );
}

function drawButterflyAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  for (let index = 0; index < 2; index += 1) {
    const orbit = frame / 13 + index * Math.PI;
    const px = spot.x + Math.cos(orbit) * (12 + index * 5);
    const py = spot.y - 12 + Math.sin(orbit * 1.4) * 7;
    const open = Math.sin(frame / 3 + index) > 0 ? 2.6 : 1.6;
    fillPixelRect(
      args.ctx,
      px - open,
      py,
      open,
      2.4,
      index === 0 ? "#F9A8D4" : "#93C5FD",
      alphaForMotion(0.76, args.reducedMotion),
    );
    fillPixelRect(
      args.ctx,
      px + 0.8,
      py,
      open,
      2.4,
      index === 0 ? "#F9A8D4" : "#93C5FD",
      alphaForMotion(0.76, args.reducedMotion),
    );
  }
}

function drawSootAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  for (let index = 0; index < 4; index += 1) {
    const scatter = ((frame * 0.9 + index * 19) % 56) / 56;
    const angle = index * (TAU / 4) + scatter * 1.4;
    fillCircle(
      args.ctx,
      spot.x + Math.cos(angle) * (5 + scatter * 12),
      spot.y - 4 + Math.sin(angle) * (3 + scatter * 6),
      1.6,
      "#1E293B",
      alphaForMotion((1 - scatter) * 0.7, args.reducedMotion),
    );
  }
}

function drawCelebrationAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  const colors = ["#FDE68A", "#F9A8D4", "#93C5FD", "#86EFAC"];
  for (let index = 0; index < 5; index += 1) {
    const rise = ((frame * 1.1 + index * 13) % 48) / 48;
    const angle = index * (TAU / 5);
    drawSpark(
      args.ctx,
      spot.x + Math.cos(angle) * (4 + rise * 14),
      spot.y - 8 - rise * 18,
      1.4,
      colors[index % colors.length],
      alphaForMotion((1 - rise) * 0.8, args.reducedMotion),
    );
  }
}

function drawRuneAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  for (let index = 0; index < 3; index += 1) {
    const orbit = frame / 19 + index * (TAU / 3);
    const px = spot.x + Math.cos(orbit) * 13;
    const py = spot.y - 8 + Math.sin(orbit) * 4.6;
    const from: Point = { x: px - 2.4, y: py };
    const to: Point = { x: px + 2.4, y: py };
    strokeLine(
      args.ctx,
      from,
      to,
      index % 2 === 0 ? "#60A5FA" : "#A78BFA",
      1.4,
      alphaForMotion(0.6, args.reducedMotion),
    );
  }
}

function drawMailboxAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
): void {
  const pop = Math.abs(wave(frame, 16, 0, 2.6, args.reducedMotion));
  drawPixelText(
    args.ctx,
    "!",
    spot.x + 10,
    spot.y - 18 - pop,
    "#FDE68A",
    alphaForMotion(0.84, args.reducedMotion),
  );
}

function drawFishingAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const progress = heldProgress(heldMs, 13_000);
  const rodBase: Point = { x: spot.x + 7, y: spot.y - 6 };
  const rodTip: Point = { x: spot.x + 22, y: spot.y - 22 };
  strokeLine(args.ctx, rodBase, rodTip, "#6B4F3A", 1.6, 0.92);
  const biting = progress > 0.55 && progress < 0.8;
  const dipAmp = biting ? 3.4 : 1.2;
  const dip = Math.abs(
    wave(frame, biting ? 5 : 16, 0, dipAmp, args.reducedMotion),
  );
  const bobberY = spot.groundY + 3 + dip;
  const bobberX = spot.x + 25;
  strokeLine(
    args.ctx,
    rodTip,
    { x: bobberX, y: bobberY - 2 },
    "#E2E8F0",
    0.8,
    0.6,
  );
  fillCircle(args.ctx, bobberX, bobberY, 2, "#EF4444", 0.95);
  fillCircle(args.ctx, bobberX, bobberY - 1, 1, "#F8FAFC", 0.9);
  const ringT = ((frame % 50) / 50 + dip * 0.04) % 1;
  strokeEllipse(
    args.ctx,
    bobberX,
    bobberY + 2,
    3 + ringT * 9,
    1 + ringT * 2.6,
    "#7DD3FC",
    1,
    alphaForMotion((1 - ringT) * (biting ? 0.5 : 0.3), args.reducedMotion),
  );
  if (progress >= 0.8) {
    const arc = (frame % 44) / 44;
    const fx = bobberX - arc * 14;
    const fy = bobberY - Math.sin(arc * Math.PI) * 15;
    fillEllipse(args.ctx, fx, fy, 3.4, 1.8, "#FB923C", 0.95);
    fillPixelRect(args.ctx, fx - 3.8, fy - 1, 2, 2, "#FDBA74", 0.9);
    fillPixelRect(args.ctx, fx + 2, fy - 0.6, 1, 1, "#1E293B", 0.95);
    if (arc < 0.2 || arc > 0.85) {
      drawSpark(
        args.ctx,
        bobberX - 6,
        bobberY - 2,
        1.6,
        "#BAE6FD",
        alphaForMotion(0.7, args.reducedMotion),
      );
    }
    drawSpark(
      args.ctx,
      spot.x + 12,
      spot.y - 26,
      1.6,
      "#FDE68A",
      alphaForMotion(0.75, args.reducedMotion),
    );
  }
}

function drawCairnAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const stage = heldStage(heldMs, 3_000, 5);
  const cx = spot.x + 18;
  const sizes = [6.4, 5.2, 4, 2.8];
  let topY = spot.groundY + 2;
  for (let index = 0; index <= Math.min(stage, 3); index += 1) {
    const settling = index === stage && stage < 4;
    const wobble = settling
      ? wave(frame, 7, index, 1.4, args.reducedMotion)
      : 0;
    const radius = sizes[index];
    topY -= radius * 1.1;
    fillEllipse(
      args.ctx,
      cx + wobble,
      topY,
      radius,
      radius * 0.72,
      index % 2 === 0 ? "#94A3B8" : "#A8998A",
      0.95,
    );
    fillEllipse(
      args.ctx,
      cx + wobble - radius * 0.3,
      topY - radius * 0.24,
      radius * 0.4,
      radius * 0.2,
      "#C3B5A2",
      0.6,
    );
    topY -= radius * 0.5;
  }
  if (stage >= 4) {
    drawSpark(
      args.ctx,
      cx + 6,
      topY - 4 + wave(frame, 20, 0, 1.4, args.reducedMotion),
      1.7,
      "#FDE68A",
      alphaForMotion(0.8, args.reducedMotion),
    );
  } else if (heldMs > 400 && (frame % 36 | 0) < 5) {
    fillEllipse(
      args.ctx,
      cx,
      spot.groundY + 2,
      5,
      1.4,
      "#C9BFAE",
      alphaForMotion(0.4, args.reducedMotion),
    );
  }
}

function drawFireflyJarAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const progress = heldProgress(heldMs, 11_000);
  const jx = spot.x + 15;
  const jy = spot.groundY - 5;
  fillCircle(
    args.ctx,
    jx,
    jy,
    7 + progress * 5,
    "#FDE68A",
    alphaForMotion(0.05 + progress * 0.16, args.reducedMotion),
  );
  strokeLine(
    args.ctx,
    { x: jx - 3.4, y: jy - 4 },
    { x: jx - 3.4, y: jy + 4 },
    "#BAE6FD",
    1.1,
    0.7,
  );
  strokeLine(
    args.ctx,
    { x: jx + 3.4, y: jy - 4 },
    { x: jx + 3.4, y: jy + 4 },
    "#BAE6FD",
    1.1,
    0.7,
  );
  strokeEllipse(args.ctx, jx, jy + 4, 3.4, 1.4, "#BAE6FD", 1.1, 0.7);
  fillPixelRect(args.ctx, jx - 3.8, jy - 5.4, 7.6, 1.8, "#A16207", 0.9);
  const caught = Math.round(progress * 4);
  for (let index = 0; index < caught; index += 1) {
    drawSpark(
      args.ctx,
      jx + wave(frame, 9 + index, index * 2.2, 2, args.reducedMotion),
      jy + wave(frame, 12 + index, index, 2.2, args.reducedMotion),
      1.2,
      "#FDE68A",
      alphaForMotion(0.85, args.reducedMotion),
    );
  }
  const loose = countForMotion(3, args.compact, args.reducedMotion);
  for (let index = 0; index < loose; index += 1) {
    const converge = clamp01(progress * 1.3 - index * 0.18);
    const orbit = frame / (14 + index * 3) + index * 2.4;
    const dist = lerp(22, 4, converge);
    drawSpark(
      args.ctx,
      jx + Math.cos(orbit) * dist,
      jy - 6 + Math.sin(orbit * 1.3) * dist * 0.5,
      1.3,
      "#FDE68A",
      alphaForMotion(0.6, args.reducedMotion),
    );
  }
}

function drawEaselAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const stage = heldStage(heldMs, 2_800, 5);
  const ex = spot.x + 18;
  const baseY = spot.groundY + 3;
  strokeLine(
    args.ctx,
    { x: ex - 5, y: baseY },
    { x: ex, y: baseY - 18 },
    "#6B4F3A",
    1.4,
    0.92,
  );
  strokeLine(
    args.ctx,
    { x: ex + 5, y: baseY },
    { x: ex, y: baseY - 18 },
    "#6B4F3A",
    1.4,
    0.92,
  );
  fillPixelRect(args.ctx, ex - 6, baseY - 16, 12, 9, "#FBF3E2", 0.96);
  if (stage >= 1) {
    fillPixelRect(args.ctx, ex - 5, baseY - 10, 10, 2.4, "#74B06A", 0.85);
  }
  if (stage >= 2) {
    fillPixelRect(args.ctx, ex - 5, baseY - 15, 10, 3, "#7DB8E8", 0.8);
  }
  if (stage >= 3) {
    fillPixelRect(args.ctx, ex - 3, baseY - 9.4, 1.6, 1.6, "#F9A8D4", 0.95);
    fillPixelRect(args.ctx, ex + 1.6, baseY - 9, 1.6, 1.6, "#F9A8D4", 0.95);
  }
  if (stage >= 4) {
    fillCircle(args.ctx, ex + 3, baseY - 13.6, 1.4, "#FBBF24", 0.95);
    drawSpark(
      args.ctx,
      ex + 9,
      baseY - 20 + wave(frame, 18, 0, 1.6, args.reducedMotion),
      1.5,
      "#FDE68A",
      alphaForMotion(0.7, args.reducedMotion),
    );
  }
  const dab = Math.abs(wave(frame, 11, 0, 2.4, args.reducedMotion));
  fillPixelRect(
    args.ctx,
    spot.x + 8 + dab,
    spot.y - 8 - dab * 0.5,
    1.6,
    3.4,
    "#A16207",
    0.9,
  );
}

function drawPicnicAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const progress = heldProgress(heldMs, 9_000);
  const bx = spot.x + 15;
  const by = spot.groundY + 1;
  fillEllipse(args.ctx, bx, by, 11, 3.6, "#DC2626", 0.62);
  for (let index = 0; index < 3; index += 1) {
    fillPixelRect(
      args.ctx,
      bx - 7 + index * 6,
      by - 1.6,
      2.6,
      1.4,
      "#FBF3E2",
      0.5,
    );
    fillPixelRect(
      args.ctx,
      bx - 4 + index * 6,
      by + 0.6,
      2.6,
      1.4,
      "#FBF3E2",
      0.5,
    );
  }
  if (progress < 0.85) {
    fillPixelRect(args.ctx, bx - 2, by - 4, 5, 2.4, "#FBF3E2", 0.95);
    fillPixelRect(args.ctx, bx - 2, by - 2.6, 5, 1, "#FB923C", 0.95);
  }
  const crumb = (frame % 26) / 26;
  fillPixelRect(
    args.ctx,
    bx + 4 + crumb * 4,
    by - 5 + crumb * 4,
    1,
    1,
    "#E2CFAE",
    alphaForMotion((1 - crumb) * 0.8, args.reducedMotion),
  );
  if (progress > 0.55) {
    const hopT = (frame % 64) / 64;
    const hop = Math.abs(Math.sin(hopT * Math.PI * 2)) * 2;
    const birdX = bx + 12 - clamp01((progress - 0.55) * 3) * 5;
    fillEllipse(args.ctx, birdX, by - 2 - hop, 2.6, 1.9, "#8E99A8", 0.95);
    fillCircle(args.ctx, birdX + 2.2, by - 3.4 - hop, 1.3, "#8E99A8", 0.95);
    fillPixelRect(
      args.ctx,
      birdX + 3.4,
      by - 3.6 - hop,
      1.2,
      0.8,
      "#F59E0B",
      0.95,
    );
    if (hopT > 0.7) {
      fillPixelRect(args.ctx, birdX + 2, by - 1.2, 1, 1, "#E2CFAE", 0.8);
    }
  }
}

function drawFeedCareAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const progress = heldProgress(heldMs, 6_000);
  const bx = spot.x - 16;
  const by = spot.groundY + 1;
  fillEllipse(args.ctx, bx, by + 1.4, 7.4, 2.8, "#7C3A12", 0.95);
  fillEllipse(args.ctx, bx, by, 6.6, 2.2, "#92400E", 0.97);
  const fill = 1 - progress;
  if (fill > 0.08) {
    fillEllipse(
      args.ctx,
      bx,
      by - 1 - fill * 1.6,
      5.4 * Math.max(0.34, fill),
      1.6 + fill * 1.2,
      "#FDBA74",
      0.96,
    );
    fillPixelRect(
      args.ctx,
      bx - 2,
      by - 2.4 - fill * 1.6,
      1.6,
      1,
      "#FB923C",
      0.9,
    );
    fillPixelRect(
      args.ctx,
      bx + 1,
      by - 2 - fill * 1.6,
      1.6,
      1,
      "#F59E0B",
      0.9,
    );
    const steamCount = countForMotion(3, args.compact, args.reducedMotion);
    for (let index = 0; index < steamCount; index += 1) {
      const rise = ((frame * 0.7 + index * 23) % 64) / 64;
      fillPixelRect(
        args.ctx,
        bx -
          3 +
          index * 3 +
          wave(frame, 13 + index, index, 1.4, args.reducedMotion),
        by - 6 - rise * 13,
        1.2,
        2.2,
        "#E2E8F0",
        alphaForMotion((1 - rise) * 0.4 * fill, args.reducedMotion),
      );
    }
  }
  const nib = (frame % 22) / 22;
  if (progress > 0.08 && progress < 0.92) {
    fillPixelRect(
      args.ctx,
      bx + 5 + nib * 5,
      by - 3 - Math.sin(nib * Math.PI) * 5,
      1.2,
      1.2,
      "#FDBA74",
      alphaForMotion((1 - nib) * 0.85, args.reducedMotion),
    );
  }
  if (progress >= 0.92) {
    drawPixelText(
      args.ctx,
      "♥",
      bx + 2,
      by - 11 + wave(frame, 16, 0, 1.6, args.reducedMotion),
      "#F472B6",
      alphaForMotion(0.9, args.reducedMotion),
    );
  }
}

function drawPlayCareAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const progress = heldProgress(heldMs, 7_000);
  const arc = (frame % 38) / 38;
  const reach = 20;
  const bxStart = spot.x + 6;
  const ballX = bxStart + Math.abs(Math.sin(arc * Math.PI)) * 0 + arc * reach;
  const ballY = spot.groundY - Math.abs(Math.sin(arc * Math.PI * 2)) * 12;
  fillCircle(args.ctx, ballX, ballY - 2, 2.6, "#F87171", 0.96);
  fillCircle(args.ctx, ballX - 0.8, ballY - 2.8, 0.9, "#FECACA", 0.9);
  if (arc > 0.46 && arc < 0.56) {
    fillEllipse(
      args.ctx,
      ballX,
      spot.groundY + 1,
      3.4,
      1.2,
      "#C9BFAE",
      alphaForMotion(0.5, args.reducedMotion),
    );
  }
  strokeEllipse(
    args.ctx,
    ballX,
    spot.groundY + 2,
    2.4,
    0.9,
    "#92400E",
    0.8,
    alphaForMotion(0.25, args.reducedMotion),
  );
  if (progress > 0.8) {
    const colors = ["#FDE68A", "#F9A8D4", "#93C5FD", "#86EFAC"];
    for (let index = 0; index < 4; index += 1) {
      const rise = ((frame * 1.2 + index * 11) % 42) / 42;
      drawSpark(
        args.ctx,
        spot.x + Math.cos(index * (TAU / 4)) * (5 + rise * 12),
        spot.y - 10 - rise * 14,
        1.4,
        colors[index],
        alphaForMotion((1 - rise) * 0.85, args.reducedMotion),
      );
    }
  }
}

function drawCleanCareAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const progress = heldProgress(heldMs, 6_500);
  const suds =
    progress < 0.7 ? clamp01(progress * 2) : clamp01((1 - progress) * 3.4);
  const count = countForMotion(6, args.compact, args.reducedMotion);
  for (let index = 0; index < count; index += 1) {
    const rise = ((frame * 0.6 + index * 19) % 70) / 70;
    const px =
      spot.x +
      (seededUnit(601, index) - 0.5) * 26 +
      wave(frame, 15 + index, index, 2, args.reducedMotion);
    fillCircle(
      args.ctx,
      px,
      spot.y + 4 - rise * 22,
      1.6 + seededUnit(607, index) * 2,
      index % 2 === 0 ? "#E0F2FE" : "#F1F5F9",
      alphaForMotion((1 - rise) * 0.66 * suds, args.reducedMotion),
    );
  }
  fillEllipse(
    args.ctx,
    spot.x,
    spot.groundY + 3,
    14,
    3,
    "#BAE6FD",
    alphaForMotion(0.3 * suds, args.reducedMotion),
  );
  if (progress > 0.74) {
    const shine = clamp01((progress - 0.74) * 4);
    drawSpark(
      args.ctx,
      spot.x - 10,
      spot.y - 14 + wave(frame, 14, 0, 1.4, args.reducedMotion),
      1.7,
      "#FDE68A",
      alphaForMotion(0.85 * shine, args.reducedMotion),
    );
    drawSpark(
      args.ctx,
      spot.x + 11,
      spot.y - 8,
      1.4,
      "#BAE6FD",
      alphaForMotion(0.7 * shine, args.reducedMotion),
    );
  }
}

function drawSleepCareAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const progress = heldProgress(heldMs, 8_000);
  fillEllipse(
    args.ctx,
    spot.x,
    spot.y + 2,
    18,
    6,
    "#312E81",
    alphaForMotion(0.12 + progress * 0.08, args.reducedMotion),
  );
  fillEllipse(args.ctx, spot.x - 13, spot.groundY - 1, 4.6, 2, "#E0E7FF", 0.8);
  fillEllipse(
    args.ctx,
    spot.x - 13,
    spot.groundY - 1.8,
    3.4,
    1.2,
    "#C7D2FE",
    0.7,
  );
  for (let index = 0; index < 3; index += 1) {
    const rise = ((frame * 0.4 + index * 26) % 78) / 78;
    drawPixelText(
      args.ctx,
      "z",
      spot.x + 10 + rise * 8 + index * 2,
      spot.y - 16 - rise * 18,
      "#C7D2FE",
      alphaForMotion((1 - rise) * 0.7, args.reducedMotion),
    );
  }
}

function drawPetCareAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const progress = heldProgress(heldMs, 5_000);
  const count = countForMotion(5, args.compact, args.reducedMotion);
  for (let index = 0; index < count; index += 1) {
    const rise = ((frame * 0.8 + index * 17) % 56) / 56;
    const angle = index * (TAU / count);
    drawPixelText(
      args.ctx,
      "♥",
      spot.x + Math.cos(angle) * (8 + rise * 10),
      spot.y - 6 - rise * 20,
      index % 2 === 0 ? "#F472B6" : "#F9A8D4",
      alphaForMotion((1 - rise) * 0.85, args.reducedMotion),
    );
  }
  fillCircle(
    args.ctx,
    spot.x,
    spot.y - 4,
    14 + progress * 5,
    "#F9A8D4",
    alphaForMotion(0.06 + progress * 0.05, args.reducedMotion),
  );
}

function drawAcornAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const stage = heldStage(heldMs, 3_200, 4);
  const pileX = spot.x + 15;
  const pileY = spot.groundY + 1;
  if (stage < 3) {
    const drop = (frame % 46) / 46;
    fillEllipse(
      args.ctx,
      spot.x - 7 + drop * 5,
      spot.groundY - 17 + drop * 15,
      1.5,
      1.9,
      "#A16207",
      alphaForMotion((1 - drop) * 0.85, args.reducedMotion),
    );
  }
  const acornSpots = [
    { dx: 0, dy: 0 },
    { dx: -3.4, dy: 0.8 },
    { dx: 3.2, dy: 0.9 },
    { dx: -1.2, dy: -2.2 },
  ] as const;
  for (let index = 0; index <= Math.min(stage, 3); index += 1) {
    const acorn = acornSpots[index];
    const ax = pileX + acorn.dx;
    const ay = pileY + acorn.dy;
    fillEllipse(args.ctx, ax, ay, 1.7, 2, "#B45309", 0.95);
    fillEllipse(args.ctx, ax, ay - 1.7, 1.9, 1, "#6B4F3A", 0.95);
    fillPixelRect(args.ctx, ax - 0.4, ay - 3, 0.8, 1, "#4A362A", 0.9);
  }
  if (stage >= 3) {
    const lift = Math.abs(wave(frame, 18, 0, 1.8, args.reducedMotion));
    fillEllipse(args.ctx, spot.x + 6, spot.y - 14 - lift, 1.7, 2, "#B45309");
    fillEllipse(args.ctx, spot.x + 6, spot.y - 15.8 - lift, 1.9, 1, "#6B4F3A");
    drawSpark(
      args.ctx,
      spot.x + 10,
      spot.y - 18 - lift,
      1.5,
      "#FDE68A",
      alphaForMotion(0.75, args.reducedMotion),
    );
  } else if (heldMs > 500 && (frame % 38 | 0) < 5) {
    fillEllipse(
      args.ctx,
      pileX,
      pileY + 2,
      4.6,
      1.3,
      "#C9BFAE",
      alphaForMotion(0.35, args.reducedMotion),
    );
  }
}

function drawLeafUmbrellaAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const sway = wave(frame, 30, 0, 1.2, args.reducedMotion);
  const ux = spot.x + sway * 0.4;
  const uy = spot.y - 19;
  strokeLine(
    args.ctx,
    { x: spot.x + 2, y: spot.y - 4 },
    { x: ux, y: uy + 2 },
    "#3F6B35",
    1.3,
    0.92,
  );
  fillEllipse(args.ctx, ux, uy, 11, 3.4, "#4A7D40", 0.96);
  fillEllipse(args.ctx, ux - 2, uy - 1.1, 7.4, 2, "#5C9450", 0.94);
  fillEllipse(args.ctx, ux - 4, uy - 1.8, 3.4, 1, "#79B26A", 0.9);
  strokeLine(
    args.ctx,
    { x: ux - 10.4, y: uy + 0.6 },
    { x: ux + 10.8, y: uy - 1 },
    "#3F6B35",
    0.6,
    0.7,
  );
  const dripCount = countForMotion(4, args.compact, args.reducedMotion);
  for (let index = 0; index < dripCount; index += 1) {
    const t = ((frame * 1.4 + index * 23) % 54) / 54;
    const side = index % 2 === 0 ? -1 : 1;
    const edgeX = ux + side * (10 + seededUnit(911, index) * 1.6);
    fillPixelRect(
      args.ctx,
      edgeX + side * t * 2.4,
      uy + 1 + t * t * 17,
      1.2,
      2.2,
      "#7DD3FC",
      alphaForMotion((1 - t) * 0.72, args.reducedMotion),
    );
  }
  if (heldProgress(heldMs, 9_000) > 0.7) {
    drawPixelText(
      args.ctx,
      "♥",
      spot.x - 9,
      spot.y - 13 + wave(frame, 20, 1, 1.4, args.reducedMotion),
      "#F9A8D4",
      alphaForMotion(0.6, args.reducedMotion),
    );
  }
}

function drawOcarinaAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const progress = heldProgress(heldMs, 12_000);
  fillEllipse(args.ctx, spot.x + 5.6, spot.y - 3.4, 2.8, 1.7, "#A87B55", 0.96);
  fillEllipse(args.ctx, spot.x + 4.6, spot.y - 4.2, 1, 0.6, "#C49A6C", 0.9);
  fillPixelRect(args.ctx, spot.x + 5.4, spot.y - 3.8, 0.8, 0.8, "#5C4332", 0.9);
  fillPixelRect(args.ctx, spot.x + 7, spot.y - 3.2, 0.7, 0.7, "#5C4332", 0.85);
  const noteCount = countForMotion(
    progress > 0.45 ? 4 : 2,
    args.compact,
    args.reducedMotion,
  );
  for (let index = 0; index < noteCount; index += 1) {
    const rise = ((frame * 0.7 + index * 24) % 76) / 76;
    drawPixelText(
      args.ctx,
      index % 2 === 0 ? "♪" : "♫",
      spot.x +
        9 +
        rise * 10 +
        wave(frame, 15 + index, index, 2.4, args.reducedMotion),
      spot.y - 10 - rise * 22,
      index % 2 === 0 ? "#C7D2FE" : "#FDE68A",
      alphaForMotion((1 - rise) * 0.8, args.reducedMotion),
    );
  }
  const listeners = Math.round(progress * 3);
  for (let index = 0; index < listeners; index += 1) {
    const orbit = frame / (22 + index * 4) + index * 2.1;
    const dist = lerp(20, 9, clamp01(progress * 1.2 - index * 0.2));
    drawSpark(
      args.ctx,
      spot.x - 6 + Math.cos(orbit) * dist,
      spot.y - 8 + Math.sin(orbit * 1.3) * dist * 0.4,
      1.3,
      "#FDE68A",
      alphaForMotion(0.66, args.reducedMotion),
    );
  }
}

function drawSeedRitualAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const grow = heldProgress(heldMs, 13_000);
  const px = spot.x + 16;
  const baseY = spot.groundY + 1;
  fillEllipse(args.ctx, px, baseY + 1, 5.4, 1.9, "#92400E", 0.9);
  const ringPulse = (frame % 52) / 52;
  strokeEllipse(
    args.ctx,
    px,
    baseY + 1,
    6 + ringPulse * 7,
    2 + ringPulse * 2.2,
    "#A7F3D0",
    1,
    alphaForMotion((1 - ringPulse) * 0.4 * (0.4 + grow), args.reducedMotion),
  );
  const stemH = 2 + grow * 13;
  fillPixelRect(args.ctx, px - 0.9, baseY - stemH, 1.8, stemH, "#16A34A", 0.92);
  const sway = wave(frame, 24, 0, 1 + grow * 1.6, args.reducedMotion);
  if (grow > 0.25) {
    fillEllipse(
      args.ctx,
      px - 3 + sway,
      baseY - stemH * 0.62,
      2.4 + grow * 2,
      1.2 + grow,
      "#4ADE80",
      0.9,
    );
    fillEllipse(
      args.ctx,
      px + 3 + sway,
      baseY - stemH * 0.82,
      2.2 + grow * 1.8,
      1.1 + grow * 0.9,
      "#86EFAC",
      0.88,
    );
  }
  if (grow > 0.62) {
    fillEllipse(
      args.ctx,
      px + sway,
      baseY - stemH - 2,
      3.2 + grow * 2.4,
      2.2 + grow * 1.6,
      "#34D399",
      0.92,
    );
  }
  if (grow >= 1) {
    for (let index = 0; index < 5; index += 1) {
      const burst = ((frame * 1.1 + index * 14) % 50) / 50;
      const angle = index * (TAU / 5);
      drawSpark(
        args.ctx,
        px + Math.cos(angle) * (4 + burst * 12),
        baseY - stemH - 4 - burst * 10,
        1.4,
        index % 2 === 0 ? "#FDE68A" : "#A7F3D0",
        alphaForMotion((1 - burst) * 0.85, args.reducedMotion),
      );
    }
  }
}

function drawSpinTopAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
  heldMs: number,
): void {
  const progress = heldProgress(heldMs, 11_000);
  const wobbleAmp = progress > 0.85 ? 2.6 : 1;
  const wobble = wave(frame, 7, 0, wobbleAmp, args.reducedMotion);
  const tx = spot.x + 14 + wobble;
  const ty = spot.groundY - 2.6;
  strokeEllipse(
    args.ctx,
    spot.x + 14,
    spot.groundY + 1,
    4.4,
    1.4,
    "#C9BFAE",
    1,
    alphaForMotion(0.35, args.reducedMotion),
  );
  fillEllipse(args.ctx, tx, ty, 3.4, 2, "#C98A5B", 0.96);
  fillEllipse(args.ctx, tx, ty - 1.4, 2.2, 1, "#E0B083", 0.92);
  fillPixelRect(args.ctx, tx - 0.5, ty - 3.4, 1, 1.6, "#8A5A36", 0.95);
  const streak = (frame % 14) / 14;
  strokeEllipse(
    args.ctx,
    tx,
    ty,
    3.8 + streak * 2,
    2.1 + streak * 0.8,
    "#F1F5F9",
    0.8,
    alphaForMotion((1 - streak) * 0.4, args.reducedMotion),
  );
  strokeLine(
    args.ctx,
    { x: tx, y: ty + 2 },
    { x: tx + wobble * 0.4, y: spot.groundY + 0.8 },
    "#8A5A36",
    1.1,
    0.9,
  );
  if (progress > 0.5 && (frame % 30 | 0) < 4) {
    drawSpark(
      args.ctx,
      tx + 5,
      ty - 5,
      1.4,
      "#FDE68A",
      alphaForMotion(0.7, args.reducedMotion),
    );
  }
}

const ACCENTS: Record<string, AccentDrawer> = {
  gather_acorns: drawAcornAccent,
  leaf_umbrella_rain: drawLeafUmbrellaAccent,
  play_ocarina: drawOcarinaAccent,
  seed_ritual: drawSeedRitualAccent,
  spin_top: drawSpinTopAccent,
  visit_pond: drawPondAccent,
  koi_pond_watch: drawPondAccent,
  warm_by_fire: drawFireAccent,
  campfire_story: drawFireAccent,
  splash_puddles: drawPuddleAccent,
  play_in_snow: drawSnowAccent,
  snow_sculpting: drawSnowAccent,
  collect_leaves: drawLeafAccent,
  leaf_storm_play: drawLeafAccent,
  smell_flowers: drawFlowerAccent,
  tend_garden: drawGardenAccent,
  nap_under_tree: drawNapAccent,
  komorebi_nap: drawNapAccent,
  rest_home: drawNapAccent,
  greet_kodama: drawKodamaAccent,
  chase_butterfly: drawButterflyAccent,
  firefly_meadow_chase: drawCelebrationAccent,
  chase_soot_sprites: drawSootAccent,
  celebrate_recovery: drawCelebrationAccent,
  receive_affection: drawCelebrationAccent,
  aurora_dance: drawCelebrationAccent,
  channel_runtime: drawRuneAccent,
  stabilize_crystal: drawRuneAccent,
  inspect_provider: drawRuneAccent,
  inspect_memory: drawRuneAccent,
  shelve_memory: drawRuneAccent,
  check_mailbox: drawMailboxAccent,
  fish_at_pond: drawFishingAccent,
  build_cairn: drawCairnAccent,
  catch_fireflies: drawFireflyJarAccent,
  paint_meadow: drawEaselAccent,
  picnic_snack: drawPicnicAccent,
  care_feed: drawFeedCareAccent,
  care_play: drawPlayCareAccent,
  care_clean: drawCleanCareAccent,
  care_sleep: drawSleepCareAccent,
  care_pet: drawPetCareAccent,
};

function drawContactShadow(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  traveling: boolean,
): void {
  const frame = safeFrame(args.frame);
  const hop = traveling
    ? Math.abs(Math.sin(frame / 3.6)) * 0.3
    : Math.abs(wave(frame, 30, 0, 0.06, args.reducedMotion));
  const radius = (args.compact ? 30 : 40) * (1 - hop * 0.25);
  fillEllipse(
    args.ctx,
    spot.x,
    spot.groundY + 4,
    radius,
    radius * 0.22,
    "#041412",
    alphaForMotion(0.26 - hop * 0.08, args.reducedMotion),
  );
  fillEllipse(
    args.ctx,
    spot.x,
    spot.groundY + 2,
    radius * 0.7,
    radius * 0.14,
    "#4ADE80",
    alphaForMotion(0.1, args.reducedMotion),
  );
}

export function drawBuddyWorldActor(
  args: DrawBuddyWorldBaseArgs,
  actor: BuddyWorldActor,
): void {
  const spot = actorSpot(args, actor);
  const traveling = travelProgress(actor) < 1;

  drawContactShadow(args, spot, traveling);

  if (traveling) {
    drawTravelDust(args, actor, spot);
    return;
  }

  if (!actor.intentKind) return;
  const accent = ACCENTS[actor.intentKind];
  if (!accent) return;
  const heldMs = Math.max(
    0,
    finiteOr(actor.nowMs, 0) -
      finiteOr(actor.intentStartedAtMs, finiteOr(actor.nowMs, 0)),
  );
  accent(args, spot, safeFrame(args.frame), heldMs);
}
