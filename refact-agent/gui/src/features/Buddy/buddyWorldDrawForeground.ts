import {
  alphaForMotion,
  countForMotion,
  fillCircle,
  fillEllipse,
  pctX,
  pctY,
  safeDimension,
  safeFrame,
  seededUnit,
  strokeBezier,
  worldPhase,
  type DrawBuddyWorldBaseArgs,
} from "./buddyWorldDrawHelpers";

const FOREGROUND_TUFT_CLUSTERS = [
  { x: 12, y: 96, spread: 9, blades: 5, seedBase: 11 },
  { x: 30, y: 97.5, spread: 7, blades: 4, seedBase: 37 },
  { x: 86, y: 96.5, spread: 10, blades: 6, seedBase: 67 },
] as const;

const PHASE_TUFT_COLORS: Record<string, [string, string]> = {
  morning: ["#14532D", "#166534"],
  day: ["#166534", "#15803D"],
  evening: ["#14442B", "#14532D"],
  night: ["#0E2A1D", "#123524"],
};

function drawForegroundTufts(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const [darkColor, lightColor] =
    PHASE_TUFT_COLORS[worldPhase(args.world)] ?? PHASE_TUFT_COLORS.day;

  for (const cluster of FOREGROUND_TUFT_CLUSTERS) {
    const baseX = pctX(width, cluster.x);
    const baseY = pctY(height, cluster.y);
    fillEllipse(
      args.ctx,
      baseX,
      baseY + 4,
      cluster.spread * 3.1,
      7,
      darkColor,
      0.85,
    );
    for (let blade = 0; blade < cluster.blades; blade += 1) {
      const bladeSeed = cluster.seedBase + blade;
      const offsetX = (seededUnit(bladeSeed, 1) - 0.5) * cluster.spread * 4;
      const bladeHeight = 14 + seededUnit(bladeSeed, 2) * 14;
      const sway = args.reducedMotion
        ? 0
        : Math.sin(frame / (26 + (blade % 3) * 6) + blade) * 2.4;
      strokeBezier(
        args.ctx,
        { x: baseX + offsetX, y: baseY + 5 },
        { x: baseX + offsetX - 1, y: baseY - bladeHeight * 0.4 },
        { x: baseX + offsetX + sway, y: baseY - bladeHeight * 0.75 },
        { x: baseX + offsetX + sway * 1.6, y: baseY - bladeHeight },
        blade % 2 === 0 ? darkColor : lightColor,
        2.2,
        0.92,
      );
    }
  }
}

const SEASON_DRIFT_COLORS: Record<string, string> = {
  spring: "#F9A8D4",
  summer: "#FDE68A",
  autumn: "#FB923C",
  winter: "#F8FAFC",
};

function drawForegroundDrift(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const color = SEASON_DRIFT_COLORS[args.world.season] ?? "#FDE68A";
  const count = countForMotion(4, args.compact, args.reducedMotion);

  for (let index = 0; index < count; index += 1) {
    const speed = 0.5 + (index % 3) * 0.22;
    const driftX =
      ((frame * speed + index * 173 + seededUnit(91, index) * 200) %
        (width + 60)) -
      30;
    const driftY =
      ((frame * (0.3 + (index % 2) * 0.14) + index * 87) % (height + 40)) - 20;
    const wobble = args.reducedMotion ? 0 : Math.sin(frame / 18 + index) * 4;
    if (args.world.season === "winter") {
      fillCircle(
        args.ctx,
        driftX + wobble,
        driftY,
        2.6 + (index % 2),
        color,
        alphaForMotion(0.5, args.reducedMotion),
      );
    } else {
      fillEllipse(
        args.ctx,
        driftX + wobble,
        driftY,
        3.6 + (index % 2),
        2.1,
        color,
        alphaForMotion(0.45, args.reducedMotion),
      );
    }
  }
}

export function drawBuddyWorldForeground(args: DrawBuddyWorldBaseArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const { ctx } = args;
  ctx.globalAlpha = 1;
  ctx.globalCompositeOperation = "source-over";
  ctx.clearRect(0, 0, width, height);
  drawForegroundTufts(args);
  drawForegroundDrift(args);
}
