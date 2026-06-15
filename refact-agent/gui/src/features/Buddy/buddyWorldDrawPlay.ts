import {
  GIFT_MOMENT_MS,
  fetchBallPositionAt,
  type BuddyGiftMoment,
  type BuddyPlaySession,
} from "./buddyPlaySessions";
import {
  fillCircle,
  fillEllipse,
  finiteOr,
  pctX,
  pctY,
  safeDimension,
  safeFrame,
  strokeCircle,
  strokeLine,
  wave,
  type DrawBuddyWorldBaseArgs,
} from "./buddyWorldDrawHelpers";

export interface BuddyPlayDrawState {
  ball: { x: number; y: number; airborne: boolean } | null;
  fireflyTarget: { x: number; y: number } | null;
  fireflyOrbitCount: number;
  releaseBurst: boolean;
  gift: { item: BuddyGiftMoment["item"]; progress: number } | null;
}

export function buildBuddyPlayDrawState(
  session: BuddyPlaySession | null,
  gift: BuddyGiftMoment | null,
  nowMs: number,
): BuddyPlayDrawState {
  const safeNow = finiteOr(nowMs, 0);
  const state: BuddyPlayDrawState = {
    ball: null,
    fireflyTarget: null,
    fireflyOrbitCount: 0,
    releaseBurst: false,
    gift: null,
  };
  if (session?.kind === "fetch") {
    state.ball = fetchBallPositionAt(session, safeNow);
  }
  if (session?.kind === "firefly") {
    state.fireflyOrbitCount = session.catches;
    if (session.phase === "pouncing") {
      state.fireflyTarget = { x: session.targetX, y: session.targetY };
    }
    if (session.phase === "done") {
      state.releaseBurst = true;
    }
  }
  if (gift) {
    const progress = Math.max(
      0,
      Math.min(1, (safeNow - gift.startedAtMs) / GIFT_MOMENT_MS),
    );
    if (progress < 1) state.gift = { item: gift.item, progress };
  }
  return state;
}

const GIFT_COLORS: Record<BuddyGiftMoment["item"], [string, string]> = {
  acorn: ["#92400E", "#B45309"],
  fish: ["#FB923C", "#FED7AA"],
  leaf: ["#DC2626", "#F87171"],
  flower: ["#F9A8D4", "#FBCFE8"],
  spark: ["#FDE68A", "#FEF3C7"],
};

export function drawBuddyPlayEffects(
  args: DrawBuddyWorldBaseArgs,
  state: BuddyPlayDrawState,
  actorXPercent: number,
  actorYPercent: number,
): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, 260);
  const frame = safeFrame(args.frame);
  const actorX = pctX(width, finiteOr(actorXPercent, 50));
  const actorY = pctY(height, finiteOr(actorYPercent, 78));

  if (state.ball) {
    const x = pctX(width, state.ball.x);
    const y = pctY(height, state.ball.y);
    if (!state.ball.airborne) {
      fillEllipse(args.ctx, x, y + 3, 4.4, 1.4, "#1A2E20", 0.25);
    }
    const spin = args.reducedMotion ? 0 : frame / 3;
    fillCircle(args.ctx, x, y, 3.6, "#F87171", 0.96);
    fillCircle(args.ctx, x - 1.1, y - 1.2, 1.3, "#FECACA", 0.85);
    strokeLine(
      args.ctx,
      { x: x - 3.4, y: y + Math.sin(spin) * 1.2 },
      { x: x + 3.4, y: y - Math.sin(spin) * 1.2 },
      "#B91C1C",
      0.8,
      0.6,
    );
  }

  if (state.fireflyTarget) {
    const x = pctX(width, state.fireflyTarget.x);
    const y = pctY(height, state.fireflyTarget.y);
    const pulse = 0.5 + Math.abs(wave(frame, 6, 1, 0.5, args.reducedMotion));
    fillCircle(args.ctx, x, y - 6, 1.6, "#FDE68A", 0.9);
    fillCircle(args.ctx, x, y - 6, 4.2, "#FDE68A", 0.18 * pulse);
    strokeCircle(args.ctx, x, y, 5, "#FEF3C7", 0.8, 0.4 * pulse);
  }

  if (state.fireflyOrbitCount > 0) {
    for (let index = 0; index < state.fireflyOrbitCount; index += 1) {
      const angle =
        (index / Math.max(1, state.fireflyOrbitCount)) * Math.PI * 2 +
        (args.reducedMotion ? 0 : frame / 18);
      const orbitX = actorX + Math.cos(angle) * 16;
      const orbitY = actorY - 14 + Math.sin(angle) * 7;
      fillCircle(args.ctx, orbitX, orbitY, 1.4, "#FDE68A", 0.92);
      fillCircle(args.ctx, orbitX, orbitY, 3.4, "#FDE68A", 0.16);
    }
  }

  if (state.releaseBurst) {
    const burstPhase = args.reducedMotion ? 0.5 : (frame % 36) / 36;
    for (let index = 0; index < 6; index += 1) {
      const angle = (index / 6) * Math.PI * 2;
      const distance = 8 + burstPhase * 22;
      fillCircle(
        args.ctx,
        actorX + Math.cos(angle) * distance,
        actorY - 16 + Math.sin(angle) * distance * 0.6,
        1.3,
        "#FDE68A",
        Math.max(0.1, 0.9 - burstPhase * 0.8),
      );
    }
  }

  if (state.gift) {
    const [core, glow] = GIFT_COLORS[state.gift.item];
    const lift = state.gift.progress * 16;
    const fade =
      state.gift.progress < 0.75 ? 1 : 1 - (state.gift.progress - 0.75) / 0.25;
    const x = actorX + 10;
    const y = actorY - 18 - lift;
    fillCircle(args.ctx, x, y, 5.4, glow, 0.22 * fade);
    if (state.gift.item === "leaf" || state.gift.item === "flower") {
      fillEllipse(args.ctx, x, y, 2.8, 1.7, core, 0.95 * fade);
      fillEllipse(args.ctx, x + 1, y - 1, 1.4, 0.9, glow, 0.9 * fade);
    } else if (state.gift.item === "fish") {
      fillEllipse(args.ctx, x, y, 3.2, 1.6, core, 0.95 * fade);
      strokeLine(
        args.ctx,
        { x: x + 3, y },
        { x: x + 4.6, y: y - 1.4 },
        core,
        1,
        0.9 * fade,
      );
    } else {
      fillCircle(args.ctx, x, y, 2.4, core, 0.95 * fade);
      fillCircle(args.ctx, x - 0.7, y - 0.8, 0.8, glow, 0.9 * fade);
    }
  }
}
