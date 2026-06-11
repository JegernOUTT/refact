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
) => void;

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
}

function drawLeafAccent(
  args: DrawBuddyWorldBaseArgs,
  spot: ActorSpot,
  frame: number,
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
): void {
  const dig = Math.abs(Math.sin(frame / 9));
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
      alphaForMotion(0.6 * dig, args.reducedMotion),
    );
  }
  fillPixelRect(
    args.ctx,
    spot.x + 14,
    spot.groundY - 5 - dig * 2,
    2,
    4,
    "#4ADE80",
    alphaForMotion(0.62, args.reducedMotion),
  );
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

const ACCENTS: Record<string, AccentDrawer> = {
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
  accent(args, spot, safeFrame(args.frame));
}
