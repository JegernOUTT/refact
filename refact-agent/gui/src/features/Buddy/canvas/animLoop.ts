import {
  spawnSparks,
  spawnFloatingEmoji,
  spawnAfterimage,
  spawnSpeedLines,
  spawnGroundEffect,
  spawnOrbitingOrb,
  spawnRainbowSparks,
} from "./particles";
import {
  CANVAS_CENTER_X,
  CANVAS_CENTER_Y,
  SIGNALS,
  STAGE_SIZES,
  STATUS_POOLS,
  TOY_DEFS,
  TOY_EMOJI,
  PERSISTENT_TOY_ACTIONS,
} from "../constants";
import type {
  AnimBeat,
  BuddyAnimState,
  BuddySemanticState,
  BuddyEvent,
  IdleActionType,
  ToyType,
  ToyDef,
  SignalDef,
} from "../types";

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

function pickFrom<T>(arr: readonly T[]): T {
  return arr[Math.floor(Math.random() * arr.length)];
}

function setStatus(anim: BuddyAnimState, text: string, frames = 180): void {
  anim.statusText = text;
  anim.statusTargetOpacity = 1;
  anim.statusTimer = frames;
}

function say(anim: BuddyAnimState, text: string, extraFrames = 0): void {
  setStatus(anim, text, clamp(60 + text.length * 4 + extraFrames, 90, 300));
}

function scheduleBeat(
  anim: BuddyAnimState,
  delay: number,
  beat: Omit<AnimBeat, "at">,
): void {
  anim.beats.push({ ...beat, at: anim.frame + Math.max(1, delay) });
}

function processBeats(anim: BuddyAnimState): void {
  if (anim.beats.length === 0) return;
  const due = anim.beats.filter((b) => b.at <= anim.frame);
  if (due.length === 0) return;
  anim.beats = anim.beats.filter((b) => b.at > anim.frame);
  for (const b of due) {
    switch (b.kind) {
      case "squash":
        anim.squashTargetX = b.x ?? 1;
        anim.squashTargetY = b.y ?? 1;
        break;
      case "sparks":
        spawnSparks(anim, b.count ?? 6, b.color);
        break;
      case "rainbow":
        spawnRainbowSparks(anim, b.count ?? 12);
        break;
      case "emoji":
        spawnFloatingEmoji(
          anim,
          b.emoji ?? "✨",
          b.x === undefined
            ? undefined
            : CANVAS_CENTER_X + anim.walkOffsetX + b.x,
          b.y === undefined ? undefined : CANVAS_CENTER_Y + b.y,
          b.count ?? 1,
        );
        break;
      case "impact":
        spawnGroundEffect(
          anim,
          "impact",
          CANVAS_CENTER_X + anim.walkOffsetX + (b.x ?? 0),
          CANVAS_CENTER_Y + 12,
        );
        break;
      case "dust": {
        const n = b.count ?? 1;
        for (let i = 0; i < n; i++) {
          spawnGroundEffect(
            anim,
            "dust",
            CANVAS_CENTER_X + anim.walkOffsetX + (Math.random() - 0.5) * 18,
            CANVAS_CENTER_Y + 10 + Math.random() * 3,
          );
        }
        break;
      }
      case "speedlines":
        spawnSpeedLines(anim, b.count ?? 4, b.x ?? 0, b.y ?? 0);
        break;
      case "flash":
        anim.screenFlash = Math.max(anim.screenFlash, b.x ?? 0.1);
        break;
      case "afterimage":
        spawnAfterimage(anim);
        break;
      case "status":
        say(anim, b.text ?? "", b.frames ?? 0);
        break;
      case "eyes":
        anim.eyeStyle = b.eyeStyle ?? "normal";
        anim.eyeStyleTimer = b.frames ?? 120;
        break;
    }
  }
}

function temperamentOf(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
): number {
  if (anim.temperamentSeed === 0) {
    const seed = (semantic.born % 997) / 997;
    anim.temperamentSeed = seed > 0 ? seed : 0.42;
  }
  return anim.temperamentSeed;
}

const IDLE_CHAIN_BIAS: Partial<
  Record<IdleActionType, Partial<Record<IdleActionType, number>>>
> = {
  yawn: { stretch: 3.5, nodOff: 2.5 },
  stretch: { shakeOff: 3, yawn: 1.6 },
  lookAround: { walk: 2.2, sniff: 1.8, peekAround: 1.4 },
  sniff: { walk: 1.8, lookAround: 1.5 },
  groom: { stretch: 1.5 },
  wiggle: { walk: 2.5, playDuck: 2 },
  sigh: { scratch: 1.6, daydream: 1.5 },
  shakeOff: { lookAround: 1.6 },
  daydream: { yawn: 1.6, lookAround: 1.4 },
  stumble: { lookAround: 2.5, scratch: 1.5 },
  zoomies: { sniff: 1.6, drinkCoffee: 2.2 },
  peekCamera: { wave: 2.4 },
  wave: { peekCamera: 1.5 },
  doze: { stretch: 2.5, yawn: 2 },
};

function selectIdleAction(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
): IdleActionType {
  const m = semantic.mood;
  const p = semantic.personality;
  const t = temperamentOf(anim, semantic);
  const raw: { action: IdleActionType; weight: number }[] = [
    { action: "lookAround", weight: 10 + m.curiosity * 0.2 },
    { action: "stretch", weight: 8 + (100 - m.energy) * 0.08 },
    { action: "yawn", weight: 5 + (100 - m.energy) * 0.12 },
    { action: "tap", weight: 6 },
    { action: "fidget", weight: m.anxiety * 0.35 },
    { action: "walk", weight: m.boredom * 0.18 + p.playfulness * 0.12 },
    {
      action: "playDuck",
      weight: p.playfulness > 20 ? p.playfulness * 0.14 : 0,
    },
    { action: "playDice", weight: m.curiosity > 30 ? m.curiosity * 0.1 : 0 },
    {
      action: "drinkCoffee",
      weight: m.energy < 50 ? (50 - m.energy) * 0.5 : 0,
    },
    {
      action: "playBug",
      weight: anim.errorStreak > 1 ? anim.errorStreak * 6 : 0,
    },
    { action: "readScroll", weight: semantic.progress.xp > 80 ? 7 : 0 },
    { action: "doze", weight: m.energy < 10 ? 30 : 0 },
    { action: "nodOff", weight: m.energy < 24 ? 22 : 0 },
    {
      action: "confidentPose",
      weight: p.confidence > 40 ? p.confidence * 0.09 : 0,
    },
    {
      action: "wave",
      weight:
        (m.affection > 15 ? 4 + m.affection * 0.06 : 3) + p.sociability * 0.04,
    },
    {
      action: "spin",
      weight: p.playfulness > 35 && m.energy > 45 ? 4 : 0,
    },
    { action: "type_code", weight: 7 },
    {
      action: "scratch",
      weight: 5 + m.anxiety * 0.04 + (100 - m.energy) * 0.02,
    },
    { action: "peekAround", weight: 5 + m.curiosity * 0.06 },
    { action: "sniff", weight: 4 + m.curiosity * 0.05 },
    {
      action: "sigh",
      weight:
        anim.sighCooldown === 0
          ? 3 + m.boredom * 0.1 + (100 - m.happiness) * 0.06
          : 0,
    },
    { action: "earTwitch", weight: 6 + t * 3 },
    { action: "headTiltHold", weight: 3 + m.curiosity * 0.07 },
    { action: "groom", weight: 4.5 },
    {
      action: "wiggle",
      weight:
        p.playfulness > 35 && m.energy > 45 ? 4 + p.playfulness * 0.06 : 0,
    },
    {
      action: "zoomies",
      weight:
        anim.rareActionCooldown === 0 && m.energy > 60
          ? p.playfulness * 0.07 + p.chaos * 0.08
          : 0,
    },
    { action: "shiver", weight: m.anxiety > 50 ? m.anxiety * 0.09 : 0 },
    {
      action: "daydream",
      weight: m.boredom > 25 ? 4 + m.curiosity * 0.06 : 0,
    },
    {
      action: "peekCamera",
      weight: 2.5 + p.sociability * 0.07 + m.affection * 0.04,
    },
    { action: "shakeOff", weight: 0.6 },
  ];
  const candidates = raw
    .map((c) => {
      let w = c.weight;
      const recency = anim.recentIdleActions.lastIndexOf(c.action);
      if (recency >= 0) {
        const age = anim.recentIdleActions.length - recency;
        w *= age <= 1 ? 0.15 : age === 2 ? 0.35 : 0.6;
      }
      const bias = anim.nextIdleBias[c.action];
      if (bias !== undefined) w *= bias;
      return { action: c.action, weight: w };
    })
    .filter((c) => c.weight > 0.4);

  const total = candidates.reduce((s, c) => s + c.weight, 0);
  if (total <= 0) return "lookAround";
  let r = Math.random() * total;
  for (const c of candidates) {
    r -= c.weight;
    if (r <= 0) return c.action;
  }
  return "lookAround";
}

function getIdleActionDuration(action: IdleActionType): number {
  const durations: Partial<Record<IdleActionType, number>> = {
    lookAround: 60 + Math.random() * 80,
    stretch: 55,
    yawn: 75,
    tap: 40,
    fidget: 45 + Math.random() * 40,
    walk: 999,
    playDuck: 160,
    playDice: 140,
    drinkCoffee: 150,
    playBug: 150,
    readScroll: 130,
    doze: 250 + Math.random() * 150,
    confidentPose: 90,
    wave: 80,
    spin: 55,
    type_code: 120 + Math.random() * 80,
    scratch: 65,
    peekAround: 90,
    sniff: 32,
    sigh: 48,
    earTwitch: 14,
    headTiltHold: 70 + Math.random() * 30,
    groom: 90 + Math.random() * 40,
    wiggle: 28,
    zoomies: 999,
    nodOff: 100,
    shiver: 30,
    daydream: 160 + Math.random() * 80,
    shakeOff: 22,
    peekCamera: 80,
    stumble: 44,
  };
  return durations[action] ?? 60;
}

function startWalk(anim: BuddyAnimState, semantic: BuddySemanticState): void {
  anim.walking = true;
  anim.walkLeanFrames = 5;
  anim.walkVel = 0;
  const range = 20 + semantic.mood.boredom * 0.3;
  anim.walkTargetX = (Math.random() - 0.5) * range * 2;
  anim.walkDirection = Math.sign(anim.walkTargetX - anim.walkOffsetX) || 1;
  anim.walkSpeed = 0.38 + (semantic.mood.energy / 100) * 0.5;
  anim.walkPhase = 0;
}

function stopWalk(anim: BuddyAnimState): void {
  anim.walking = false;
  anim.walkVel = 0;
  anim.idleAction = "none";
  anim.idleActionTimer = 0;
}

function startZoomies(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
): void {
  anim.idleAction = "zoomies";
  anim.idleActionTimer = 999;
  anim.idleActionTotal = 999;
  anim.zoomiesDashesLeft = 2 + Math.floor(Math.random() * 3);
  anim.rareActionCooldown = 1600;
  anim.walking = true;
  anim.walkLeanFrames = 6;
  anim.walkVel = 0;
  anim.walkSpeed = 0.55 + (semantic.mood.energy / 100) * 0.5;
  anim.walkTargetX = (Math.random() < 0.5 ? -1 : 1) * (26 + Math.random() * 14);
  anim.walkDirection = Math.sign(anim.walkTargetX - anim.walkOffsetX) || 1;
  anim.eyeStyle = "wide";
  anim.eyeStyleTimer = 90;
  say(anim, pickFrom(STATUS_POOLS.zoomies_start));
  spawnSpeedLines(anim, 4, Math.PI, 0);
}

function triggerStumble(anim: BuddyAnimState): void {
  const cx = CANVAS_CENTER_X + anim.walkOffsetX;
  anim.walking = false;
  anim.walkVel = 0;
  anim.zoomiesDashesLeft = 0;
  anim.idleAction = "stumble";
  anim.idleActionTimer = 44;
  anim.idleActionTotal = 44;
  anim.stumbleCooldown = 900;
  anim.blushTimer = 130;
  anim.squashTargetX = 1.3;
  anim.squashTargetY = 0.7;
  anim.eyeStyle = "wide";
  anim.eyeStyleTimer = 30;
  spawnFloatingEmoji(anim, "💫", cx, CANVAS_CENTER_Y - 24);
  spawnGroundEffect(anim, "dust", cx - 5, CANVAS_CENTER_Y + 11);
  spawnGroundEffect(anim, "dust", cx + 5, CANVAS_CENTER_Y + 11);
  say(anim, pickFrom(STATUS_POOLS.stumble));
}

function updateWalk(anim: BuddyAnimState, semantic: BuddySemanticState): void {
  if (!anim.walking) {
    anim.walkVel *= 0.8;
    anim.walkOffsetX *= 0.93;
    if (Math.abs(anim.walkOffsetX) < 0.4) anim.walkOffsetX = 0;
    return;
  }
  const cx = CANVAS_CENTER_X + anim.walkOffsetX;
  if (anim.walkLeanFrames > 0) {
    anim.walkLeanFrames--;
    anim.squashTargetX = 1.07;
    anim.squashTargetY = 0.95;
    if (anim.walkLeanFrames === 0)
      spawnGroundEffect(
        anim,
        "dust",
        cx - anim.walkDirection * 5,
        CANVAS_CENTER_Y + 12,
      );
    return;
  }
  const zoom = anim.zoomiesDashesLeft > 0;
  const dist = anim.walkTargetX - anim.walkOffsetX;
  const dir = Math.sign(dist) || 1;
  if (dir !== anim.walkDirection && Math.abs(dist) > 2) {
    anim.walkDirection = dir;
    anim.walkVel *= 0.3;
    anim.squashTargetX = 1.1;
    anim.squashTargetY = 0.93;
    spawnGroundEffect(anim, "dust", cx, CANVAS_CENTER_Y + 12);
  }
  const cruise = zoom ? anim.walkSpeed * 3 : anim.walkSpeed;
  const ease = Math.min(1, Math.abs(dist) / 14);
  const desired = dir * cruise * (0.3 + 0.7 * ease);
  anim.walkVel += (desired - anim.walkVel) * 0.12;
  anim.walkOffsetX += anim.walkVel;
  anim.walkPhase += 0.09 + Math.abs(anim.walkVel) * 0.09;

  if (semantic.mood.happiness > 65 && anim.frame % 22 === 0) {
    anim.squashTargetX = 1.12;
    anim.squashTargetY = 0.88;
  }
  if (anim.frame % (zoom ? 3 : 5) === 0)
    spawnGroundEffect(
      anim,
      "dust",
      cx + anim.walkDirection * 6,
      CANVAS_CENTER_Y + 13,
    );
  if (zoom) {
    if (anim.frame % 4 === 0) spawnAfterimage(anim);
    if (anim.frame % 7 === 0)
      spawnSpeedLines(anim, 2, anim.walkDirection > 0 ? Math.PI : 0, 0);
  }
  if (
    !zoom &&
    semantic.personality.chaos > 55 &&
    Math.abs(dist) > 8 &&
    Math.random() < 0.002
  ) {
    anim.walkTargetX = -anim.walkTargetX;
  }
  if (
    !zoom &&
    semantic.personality.chaos > 35 &&
    anim.stumbleCooldown === 0 &&
    Math.abs(anim.walkVel) > 0.3 &&
    Math.random() < 0.0035
  ) {
    triggerStumble(anim);
    return;
  }
  if (Math.abs(dist) < 1.4 && Math.abs(anim.walkVel) < 0.3) {
    if (zoom) {
      anim.zoomiesDashesLeft--;
      if (anim.zoomiesDashesLeft <= 0) {
        stopWalk(anim);
        anim.pantTimer = 150;
        anim.squashTargetX = 1.12;
        anim.squashTargetY = 0.9;
        say(anim, pickFrom(STATUS_POOLS.zoomies_end));
        spawnGroundEffect(anim, "dust", cx, CANVAS_CENTER_Y + 12);
      } else {
        anim.walkTargetX =
          -Math.sign(anim.walkOffsetX || 1) * (24 + Math.random() * 16);
        spawnAfterimage(anim);
      }
      return;
    }
    if (Math.random() < 0.22 && semantic.mood.curiosity > 20) {
      anim.walking = false;
      anim.walkVel = 0;
      anim.idleAction = "lookAround";
      anim.idleActionTimer = 30 + Math.floor(Math.random() * 50);
      anim.idleActionTotal = anim.idleActionTimer;
      return;
    }
    if (Math.random() < 0.38) {
      stopWalk(anim);
      return;
    }
    const range = 15 + semantic.mood.boredom * 0.25;
    anim.walkTargetX = (Math.random() - 0.5) * range * 2;
    anim.walkDirection = Math.sign(anim.walkTargetX - anim.walkOffsetX) || 1;
  }
  if (Math.abs(anim.walkOffsetX) > 44) {
    anim.walkTargetX = -anim.walkOffsetX * 0.5;
    anim.walkDirection = Math.sign(anim.walkTargetX - anim.walkOffsetX);
  }
}

function startToy(
  anim: BuddyAnimState,
  toyType: ToyType,
  emit: (e: BuddyEvent) => void,
): void {
  const def = TOY_DEFS[toyType] as ToyDef | undefined;
  if (def === undefined) return;
  anim.toyActive = true;
  anim.toyType = toyType;
  anim.toyAnimPhase = 0;
  anim.toyDurationTimer = 140 + Math.random() * 40;
  setStatus(anim, def.statusMessage, 160);
  if (def.xp > 0) emit({ type: "xp_gained", amount: def.xp, newTotal: 0 });
  spawnFloatingEmoji(
    anim,
    (TOY_EMOJI[toyType] as string | undefined) ?? "📦",
    undefined,
    CANVAS_CENTER_Y - 22,
  );
}

function stopToy(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
  emit: (e: BuddyEvent) => void,
): void {
  if (!anim.toyActive || !anim.toyType) return;
  const def = TOY_DEFS[anim.toyType];
  if (def.energyRestore) {
    emit({
      type: "semantic_update",
      patch: {
        mood: {
          ...semantic.mood,
          energy: Math.min(100, semantic.mood.energy + def.energyRestore),
        },
      },
    });
    anim.eyeStyle = "star";
    anim.eyeStyleTimer = 110;
    spawnSparks(anim, 6, "#FBBF24");
  }
  anim.toyActive = false;
  anim.toyType = null;
  anim.idleAction = "none";
  anim.idleActionTimer = 0;
}

function getCursorTrackSpeed(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
): number {
  if (anim.idleAction === "doze") return 0.008;
  if (anim.walking) return 0.022;
  if (
    semantic.activity.animationType === "work" ||
    semantic.activity.animationType === "think"
  )
    return 0.028;
  const m = semantic.mood;
  const p = semantic.personality;
  let speed = 0.08;
  speed *= 0.3 + (m.curiosity / 100) * 0.7;
  speed *= 0.4 + (m.energy / 100) * 0.6;
  speed *= 0.5 + (p.clinginess / 100) * 0.5;
  if (m.anxiety > 50) speed *= 1.4;
  if (p.confidence > 65) speed *= 0.6;
  return Math.max(0.006, Math.min(0.18, speed));
}
const GAZE_DRIVEN_ACTIONS = new Set<IdleActionType>([
  "lookAround",
  "peekAround",
  "daydream",
  "peekCamera",
  "stumble",
]);

function forceBlink(anim: BuddyAnimState, frames = 5): void {
  if (anim.blinking) return;
  anim.blinking = true;
  anim.blinkFrames = frames;
  anim.blinkTick = 0;
}

function updateGaze(anim: BuddyAnimState, semantic: BuddySemanticState): void {
  const speed =
    anim.saccadeFrames > 0 ? 0.45 : getCursorTrackSpeed(anim, semantic);
  if (anim.saccadeFrames > 0) anim.saccadeFrames--;
  anim.eyeLookX += (anim.cursorTargetX - anim.eyeLookX) * speed;
  anim.eyeLookY += (anim.cursorTargetY - anim.eyeLookY) * speed;
  if (anim.saccadeFrames === 0 && anim.frame % 9 === 0 && Math.random() < 0.5) {
    anim.eyeLookX += (Math.random() - 0.5) * 0.05;
    anim.eyeLookY += (Math.random() - 0.5) * 0.04;
  }
  if (anim.mouseProximity > 0.25) {
    const dx = anim.cursorTargetX - anim.eyeLookX;
    const dy = anim.cursorTargetY - anim.eyeLookY;
    if (Math.abs(dx) + Math.abs(dy) > 0.7 && anim.saccadeFrames === 0) {
      anim.saccadeFrames = 3;
      if (Math.random() < 0.25) forceBlink(anim, 4);
    }
    anim.gazeSettleFrames = 40;
    return;
  }
  if (GAZE_DRIVEN_ACTIONS.has(anim.idleAction)) return;
  if (anim.gazeSettleFrames > 0) {
    anim.gazeSettleFrames--;
    return;
  }
  const p = semantic.personality;
  const choices: { x: number; y: number; weight: number }[] = [
    {
      x: (Math.random() - 0.5) * 2.4,
      y: (Math.random() - 0.5) * 1.2,
      weight: 3,
    },
    {
      x: 0,
      y: 0.05,
      weight: p.sociability * 0.02 + semantic.mood.affection * 0.012,
    },
  ];
  if (anim.statusOpacity > 0.4) choices.push({ x: 0.1, y: -0.9, weight: 2 });
  if (anim.toyActive) choices.push({ x: 1.5, y: 0.4, weight: 3.5 });
  const total = choices.reduce((s, c) => s + c.weight, 0);
  let r = Math.random() * total;
  let target = choices[0];
  for (const c of choices) {
    r -= c.weight;
    if (r <= 0) {
      target = c;
      break;
    }
  }
  const jump =
    Math.abs(target.x - anim.eyeLookX) + Math.abs(target.y - anim.eyeLookY);
  anim.cursorTargetX = target.x;
  anim.cursorTargetY = target.y;
  anim.saccadeFrames = 4;
  anim.gazeSettleFrames = 50 + Math.floor(Math.random() * 140);
  if (jump > 0.9 && Math.random() < 0.3) forceBlink(anim, 4);
}

function updateBlink(anim: BuddyAnimState, semantic: BuddySemanticState): void {
  const m = semantic.mood;
  const t = temperamentOf(anim, semantic);
  if (anim.slowBlinkTimer > 0) {
    anim.slowBlinkTimer--;
    const half = 26;
    const ph = 1 - Math.abs(anim.slowBlinkTimer - half) / half;
    anim.lidClose = clamp(ph, 0, 1);
    anim.blinking = false;
    return;
  }
  const focused =
    semantic.activity.animationType === "work" ||
    semantic.activity.animationType === "think";
  let interval = 110 + t * 90;
  if (m.anxiety > 45) interval *= 0.55;
  if (focused) interval *= 1.8;
  if (m.energy < 30) interval *= 0.8;

  if (
    !anim.blinking &&
    m.affection > 45 &&
    semantic.activity.animationType === "idle" &&
    anim.idleAction !== "doze" &&
    Math.random() < 0.0012
  ) {
    anim.slowBlinkTimer = 52;
    return;
  }

  anim.blinkTick++;
  const due =
    anim.blinkTick >= anim.nextBlinkAt ||
    (anim.blinkQueue > 0 && anim.blinkTick >= 14);
  if (!anim.blinking && due) {
    anim.blinking = true;
    anim.blinkTick = 0;
    anim.blinkFrames =
      m.energy < 30
        ? 10 + Math.floor(Math.random() * 4)
        : 6 + Math.floor(Math.random() * 3);
    if (anim.blinkQueue > 0) anim.blinkQueue--;
  }
  if (anim.blinking) {
    anim.blinkFrames--;
    if (anim.blinkFrames <= 0) {
      anim.blinking = false;
      anim.nextBlinkAt = interval * (0.6 + Math.random() * 0.8);
      if (anim.blinkQueue === 0 && Math.random() < 0.22) anim.blinkQueue = 1;
    }
  }
  const droop = m.energy < 35 ? ((35 - m.energy) / 35) * 0.45 : 0;
  anim.lidBase += (droop - anim.lidBase) * 0.03;
  const target = anim.blinking ? 1 : 0;
  const lerp = target > anim.lidClose ? 0.5 : 0.22;
  anim.lidClose += (target - anim.lidClose) * lerp;
}

function updateBreath(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
): void {
  const m = semantic.mood;
  const sleeping = anim.idleAction === "doze";
  const busy =
    semantic.activity.animationType === "work" ||
    semantic.activity.animationType === "think";
  let rate = 0.028 + (m.anxiety / 100) * 0.035 + (busy ? 0.008 : 0);
  let depth = 0.0035 + (m.energy / 100) * 0.0075;
  if (sleeping) {
    rate *= 0.45;
    depth *= 1.7;
  }
  if (anim.pantTimer > 0) {
    rate = 0.16;
    depth = 0.006;
  }
  anim.breathPhase += rate;
  const s = Math.sin(anim.breathPhase);
  const shaped = s >= 0 ? Math.pow(s, 0.7) : -Math.pow(-s, 1.4);
  anim.breathScale = shaped * depth;
}

const FRIENDLY_REACTION_SIGNALS = new Set([
  "speech_humor",
  "speech_insight",
  "speech_chat_reaction",
  "chat_bug_candidate",
]);

const HABITUATION_EXEMPT = new Set([
  "chat_completed",
  "task_completed",
  "skill_learned",
  "stage_up",
  "chat_error",
  "tool_failed",
  "task_failed",
  "connection_lost",
  "user_message",
]);

function updateMoodDrift(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
  emit: (e: BuddyEvent) => void,
): void {
  const m = semantic.mood;
  const p = semantic.personality;
  const isIdling =
    semantic.activity.animationType === "idle" || anim.idleAction === "doze";

  const patch: Partial<BuddySemanticState["mood"]> = {};
  if (isIdling) {
    patch.boredom = Math.min(100, m.boredom + 0.025);
    patch.energy = Math.min(
      100,
      m.energy + (anim.idleAction === "doze" ? 0.06 : 0.018),
    );
  } else {
    patch.boredom = Math.max(0, m.boredom - 0.4);
  }
  patch.anxiety = Math.max(0, m.anxiety - (0.05 + p.resilience * 0.001));
  patch.affection = Math.max(0, m.affection - 0.04);
  if (m.happiness > 58) patch.happiness = Math.max(58, m.happiness - 0.012);
  else if (m.happiness < 48)
    patch.happiness = Math.min(48, m.happiness + 0.012);

  const personalityPatch =
    anim.mouseProximity > 0.6
      ? {
          personality: {
            ...p,
            clinginess: Math.min(100, p.clinginess + 0.006),
          },
        }
      : {};
  emit({
    type: "semantic_update",
    patch: { mood: { ...m, ...patch }, ...personalityPatch },
  });
}

function applySignalStatus(
  anim: BuddyAnimState,
  signalType: string,
  semantic?: BuddySemanticState,
): void {
  const energy = semantic?.mood.energy ?? 70;
  switch (signalType) {
    case "chat_completed":
      if (anim.successStreak >= 3)
        say(anim, pickFrom(STATUS_POOLS.chat_completed_streak));
      else if (energy < 30)
        say(anim, pickFrom(STATUS_POOLS.chat_completed_tired));
      else say(anim, pickFrom(STATUS_POOLS.chat_completed));
      break;
    case "task_completed":
      say(
        anim,
        pickFrom(
          anim.successStreak >= 3
            ? STATUS_POOLS.task_completed_streak
            : STATUS_POOLS.task_completed,
        ),
      );
      break;
    case "skill_learned":
      say(anim, pickFrom(STATUS_POOLS.skill_learned));
      break;
    case "edit_applied":
      say(anim, pickFrom(STATUS_POOLS.edit_applied));
      break;
    case "checkpoint_saved":
      say(anim, pickFrom(STATUS_POOLS.checkpoint_saved));
      break;
    case "connection_restored":
      say(anim, pickFrom(STATUS_POOLS.connection_restored));
      break;
    case "connection_lost":
      say(anim, pickFrom(STATUS_POOLS.connection_lost), 60);
      break;
    case "chat_error":
      say(anim, pickFrom(STATUS_POOLS.chat_error), 40);
      break;
    case "tool_failed":
      say(anim, pickFrom(STATUS_POOLS.tool_failed));
      break;
    case "task_failed":
      say(anim, pickFrom(STATUS_POOLS.task_failed));
      break;
    case "user_message": {
      const social = (semantic?.personality.sociability ?? 0) > 55;
      say(
        anim,
        pickFrom(
          social ? STATUS_POOLS.user_message_social : STATUS_POOLS.user_message,
        ),
      );
      break;
    }
    case "chat_started":
      say(anim, pickFrom(STATUS_POOLS.chat_started));
      break;
    default:
      break;
  }
}

export function triggerSignalAnimation(
  anim: BuddyAnimState,
  signalType: string,
  emit: (e: BuddyEvent) => void,
  semantic?: BuddySemanticState,
): void {
  const def = SIGNALS[signalType] as SignalDef | undefined;
  if (def === undefined) return;

  const wasDozing = anim.idleAction === "doze" || anim.idleAction === "nodOff";
  const prevErrorStreak = anim.errorStreak;

  if (def.isError) {
    anim.errorStreak++;
    anim.successStreak = 0;
  } else if (def.isWin) {
    anim.successStreak++;
    anim.errorStreak = Math.max(0, anim.errorStreak - 1);
  }

  if (def.isError) {
    anim.earState = -1;
  } else if (def.mood === "alert" || def.mood === "celebrate") {
    anim.earState = 1;
  } else {
    anim.earState = 0;
  }

  anim.heat = Math.min(100, anim.heat + 8);
  anim.toyActive = false;
  anim.toyType = null;
  anim.walking = false;
  anim.walkVel = 0;
  anim.zoomiesDashesLeft = 0;

  const now = Date.now();
  const priorSame = anim.signalHistory.filter(
    (s) => s.signalType === signalType && now - s.timestamp < 8000,
  ).length;
  anim.signalHistory = [
    ...anim.signalHistory.filter((s) => now - s.timestamp < 8000),
    { signalType, timestamp: now },
  ].slice(-10);

  const recent = anim.signalHistory.filter((s) => s.signalType === signalType);
  if (
    recent.length >= 3 &&
    (anim.combo.signalType !== signalType || anim.combo.count < recent.length)
  ) {
    anim.combo = {
      count: recent.length,
      signalType,
      displayTimer: 180,
      rainbowHue: 0,
    };
    const bonus = recent.length * 10;
    emit({ type: "xp_gained", amount: bonus, newTotal: 0 });
    spawnRainbowSparks(anim, 20 + recent.length * 5);
    anim.squashTargetX = 1.2;
    anim.squashTargetY = 0.82;
    anim.screenFlash = 0.14;
    spawnAfterimage(anim);
    spawnAfterimage(anim);
  }

  const energy = semantic?.mood.energy ?? 70;
  const happiness = semantic?.mood.happiness ?? 60;
  const vigor = clamp(0.55 + energy * 0.0045 + happiness * 0.0035, 0.5, 1.4);
  const habituated =
    !def.isError && !HABITUATION_EXEMPT.has(signalType) && priorSame >= 2;
  const damp = habituated ? 0.45 : 1;
  const k = (n: number): number => Math.max(1, Math.round(n * vigor * damp));
  const sq = (target: number): number =>
    1 + (target - 1) * vigor * (habituated ? 0.6 : 1);

  const isFriendlyReaction = FRIENDLY_REACTION_SIGNALS.has(signalType);
  if (!isFriendlyReaction && anim.errorStreak >= 5) {
    anim.eyeStyle = "X";
    anim.eyeStyleTimer = 240;
  } else if (!isFriendlyReaction && anim.errorStreak >= 3) {
    anim.eyeStyle = "spiral";
    anim.eyeStyleTimer = 300;
  } else if (signalType === "task_failed") {
    anim.eyeStyle = "teary";
    anim.eyeStyleTimer = 200;
  } else if (signalType === "connection_lost") {
    anim.eyeStyle = "angry";
    anim.eyeStyleTimer = 180;
  } else if (signalType === "tool_failed" || signalType === "chat_error") {
    anim.eyeStyle = "angry";
    anim.eyeStyleTimer = 120;
  } else if (signalType === "skill_learned") {
    anim.eyeStyle = "star";
    anim.eyeStyleTimer = 300;
  } else if (
    signalType === "memory_extract" ||
    signalType === "knowledge_update"
  ) {
    anim.eyeStyle = "star";
    anim.eyeStyleTimer = 180;
  } else if (anim.successStreak >= 4) {
    anim.eyeStyle = "squint";
    anim.eyeStyleTimer = 240;
  } else if (def.mood === "celebrate") {
    anim.eyeStyle = "star";
    anim.eyeStyleTimer = 150;
  } else if (def.mood === "happy") {
    anim.eyeStyle = "uwu";
    anim.eyeStyleTimer = 180;
  } else {
    anim.eyeStyle = "normal";
    anim.eyeStyleTimer = 0;
  }

  anim.moodType = def.mood;
  anim.activeScene = def.scene ?? "";
  anim.activeSceneVariant = def.animVariant ?? "";
  anim.activeSceneTimer =
    def.duration != null
      ? Math.max(120, Math.round((def.duration / 1000) * 60))
      : 300;

  const cx = CANVAS_CENTER_X + anim.walkOffsetX;
  switch (def.animationType) {
    case "celebrate":
      anim.celebrationTimer = Math.round(120 * vigor);
      anim.squashTargetX = sq(1.18);
      anim.squashTargetY = sq(0.8);
      scheduleBeat(anim, 5, { kind: "squash", x: sq(0.88), y: sq(1.16) });
      scheduleBeat(anim, 5, { kind: "speedlines", count: k(6), x: 0, y: -1 });
      scheduleBeat(anim, 6, { kind: "afterimage" });
      scheduleBeat(anim, 11, { kind: "squash", x: sq(1.16), y: sq(0.86) });
      scheduleBeat(anim, 11, { kind: "impact" });
      scheduleBeat(anim, 11, { kind: "dust", count: 2 });
      scheduleBeat(anim, 12, { kind: "sparks", count: k(14) });
      scheduleBeat(anim, 12, { kind: "flash", x: 0.12 * vigor });
      spawnFloatingEmoji(anim, def.icon, undefined, undefined, k(2));
      break;
    case "shake":
      anim.shakeIntensity = 7;
      spawnFloatingEmoji(anim, def.icon, cx, CANVAS_CENTER_Y - 24);
      anim.screenGlitch = 0.15;
      anim.squashTargetX = sq(0.9);
      anim.squashTargetY = sq(1.1);
      spawnGroundEffect(anim, "crack", cx, CANVAS_CENTER_Y + 12);
      spawnSpeedLines(anim, k(4), Math.random() * 6.28, 0);
      break;
    case "eat":
      spawnFloatingEmoji(anim, "🍕", cx + 16, CANVAS_CENTER_Y - 4);
      spawnFloatingEmoji(anim, "🍪", cx - 12, CANVAS_CENTER_Y - 8);
      spawnFloatingEmoji(anim, def.icon);
      anim.squashTargetX = sq(1.08);
      anim.squashTargetY = sq(0.93);
      break;
    case "sleep":
      anim.squashTargetX = 1.05;
      anim.squashTargetY = 0.95;
      break;
    case "think":
      spawnFloatingEmoji(anim, def.icon, cx - 16, CANVAS_CENTER_Y - 28);
      anim.squashTargetX = sq(0.96);
      anim.squashTargetY = sq(1.04);
      break;
    case "absorb":
      spawnOrbitingOrb(anim, def.icon, k(4));
      spawnSparks(anim, k(6));
      anim.screenFlash = 0.08;
      anim.squashTargetX = sq(0.93);
      anim.squashTargetY = sq(1.07);
      spawnAfterimage(anim);
      break;
    case "work":
      spawnFloatingEmoji(anim, def.icon, undefined, undefined, k(2));
      spawnOrbitingOrb(anim, "⚙️", k(3));
      spawnSpeedLines(anim, k(3), 0, -0.5);
      anim.squashTargetX = sq(1.04);
      anim.squashTargetY = sq(0.97);
      break;
    case "perk":
      spawnFloatingEmoji(anim, def.icon, undefined, undefined, k(2));
      spawnSparks(anim, k(5));
      anim.squashTargetX = sq(0.92);
      anim.squashTargetY = sq(1.08);
      anim.screenFlash = 0.06;
      spawnGroundEffect(anim, "dust", cx, CANVAS_CENTER_Y + 12);
      spawnAfterimage(anim);
      break;
  }

  if (signalType === "stage_up") {
    anim.activeSceneTimer = 360;
    anim.celebrationTimer = 360;
    spawnRainbowSparks(anim, 60);
    spawnFloatingEmoji(anim, "🌟", undefined, CANVAS_CENTER_Y - 30, 5);
    spawnFloatingEmoji(anim, "⬆", undefined, CANVAS_CENTER_Y - 20, 4);
    spawnFloatingEmoji(anim, "✨", cx - 20, CANVAS_CENTER_Y - 10, 3);
    spawnFloatingEmoji(anim, "✨", cx + 20, CANVAS_CENTER_Y - 10, 3);
    anim.squashTargetX = 1.12;
    anim.squashTargetY = 0.9;
    anim.screenFlash = 0.14;
    spawnAfterimage(anim);
    spawnAfterimage(anim);
    spawnSpeedLines(anim, 20, 0, -1);
    spawnGroundEffect(anim, "impact", cx, CANVAS_CENTER_Y + 12);
    spawnGroundEffect(anim, "dust", cx - 18, CANVAS_CENTER_Y + 10);
    spawnGroundEffect(anim, "dust", cx + 18, CANVAS_CENTER_Y + 10);
    anim.eyeStyle = "star";
    anim.eyeStyleTimer = 600;
    anim.shakeIntensity = 10;
  }

  if (!habituated || priorSame < 3) {
    applySignalStatus(anim, signalType, semantic);
  }

  if (signalType === "chat_completed" || signalType === "task_completed") {
    anim.celebrationTimer = Math.max(
      anim.celebrationTimer,
      Math.round(200 * vigor),
    );
    spawnRainbowSparks(anim, k(20));
  }
  if (signalType === "user_message") {
    anim.earState = 1;
    anim.walking = false;
    anim.walkSpeed = 0;
  }
  if (signalType === "skill_learned") {
    spawnRainbowSparks(anim, k(40));
    anim.celebrationTimer = Math.max(anim.celebrationTimer, 240);
  }
  if (signalType === "streaming" || signalType === "generating") {
    anim.heat = Math.min(100, anim.heat + 15);
  }
  if (signalType === "memory_extract" || signalType === "knowledge_update") {
    spawnOrbitingOrb(anim, "✨", 3);
  }
  if (signalType === "connection_lost") {
    anim.shakeIntensity = Math.max(anim.shakeIntensity, 5);
    anim.shiverTimer = Math.max(anim.shiverTimer, 40);
  }
  if (signalType === "speech_humor") {
    anim.blushTimer = Math.max(anim.blushTimer, 90);
    anim.eyeStyle = "wink";
    anim.eyeStyleTimer = 90;
  }
  if (signalType === "tool_failed" || signalType === "chat_error") {
    anim.cursorTargetX = 0;
    anim.cursorTargetY = 0.6;
    anim.saccadeFrames = 3;
    anim.gazeSettleFrames = 110;
  }
  if (def.isWin && prevErrorStreak >= 2 && anim.errorStreak < 2) {
    anim.nextIdleBias = { ...anim.nextIdleBias, shakeOff: 4 };
    scheduleBeat(anim, 30, {
      kind: "status",
      text: pickFrom(STATUS_POOLS.error_recovery),
    });
  }

  if (wasDozing) {
    anim.idleAction = "none";
    anim.idleActionTimer = 0;
    anim.lidClose = 0;
    anim.lidBase = 0;
    anim.eyeStyle = "wide";
    anim.eyeStyleTimer = 40;
    anim.squashTargetX = 0.85;
    anim.squashTargetY = 1.15;
    spawnFloatingEmoji(anim, "❗", cx, CANVAS_CENTER_Y - 26);
    say(anim, pickFrom(STATUS_POOLS.wake_startle));
    scheduleBeat(anim, 8, { kind: "squash", x: 1.08, y: 0.94 });
  }
}
export function updateSceneAnimation(
  anim: BuddyAnimState,
  scene: string,
  variant: string,
): void {
  switch (scene) {
    case "working":
      anim.heat = Math.min(100, anim.heat + 0.4);
      anim.eyeLookY += (0.28 - anim.eyeLookY) * 0.01;
      anim.eyeLookX += (0 - anim.eyeLookX) * 0.007;
      if (anim.frame % 20 === 0) {
        if (variant === "typing") {
          anim.squashTargetX = 1.03 + Math.sin(anim.frame * 0.3) * 0.02;
          anim.squashTargetY = 0.97 - Math.sin(anim.frame * 0.3) * 0.02;
        } else if (variant === "sorting" || variant === "building") {
          anim.squashTargetX = 1.03;
          anim.squashTargetY = 0.97;
        }
      }
      break;
    case "alert":
      if (anim.frame % 35 === 0) {
        anim.cursorTargetX = (Math.random() - 0.5) * 2.6;
        anim.cursorTargetY = (Math.random() - 0.5) * 0.6;
        anim.saccadeFrames = 3;
      }
      if (anim.frame % 40 === 0 && anim.shakeIntensity < 1) {
        anim.shakeIntensity = 1.2;
        anim.squashTargetX = 0.93;
        anim.squashTargetY = 1.07;
      }
      break;
    case "thinking":
      anim.eyeLookY += (-0.28 - anim.eyeLookY) * 0.007;
      anim.eyeLookX += (0.35 - anim.eyeLookX) * 0.004;
      anim.squashTargetX = 0.97 + Math.sin(anim.frame * 0.025) * 0.02;
      anim.squashTargetY = 1.03 - Math.sin(anim.frame * 0.025) * 0.02;
      break;
    case "celebrate":
      if (
        variant === "confetti" &&
        anim.frame % 15 === 0 &&
        anim.celebrationTimer > 0
      ) {
        anim.squashTargetX = 1.06 + Math.sin(anim.frame * 0.4) * 0.04;
        anim.squashTargetY = 0.95 - Math.sin(anim.frame * 0.4) * 0.04;
      }
      break;
    case "perk":
      if (variant === "ears_up") {
        anim.earState = Math.max(anim.earState, 0.5);
        anim.eyeLookX += (anim.cursorTargetX * 1.2 - anim.eyeLookX) * 0.04;
      } else if (variant === "curious") {
        anim.headTilt += (0.38 - anim.headTilt) * 0.05;
        anim.eyeLookX += (anim.cursorTargetX - anim.eyeLookX) * 0.04;
      }
      break;
    case "greeting":
      anim.squashTargetX = 1.04 + Math.sin(anim.frame * 0.15) * 0.03;
      anim.squashTargetY = 0.96 - Math.sin(anim.frame * 0.15) * 0.03;
      break;
    default:
      break;
  }
}

function startIdleAction(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
  emit: (e: BuddyEvent) => void,
  action: IdleActionType,
): void {
  anim.nextIdleBias = {};
  anim.idleAction = action;
  const dur = getIdleActionDuration(action) | 0;
  anim.idleActionTimer = dur;
  anim.idleActionTotal = dur;
  anim.recentIdleActions = [...anim.recentIdleActions.slice(-4), action];

  switch (action) {
    case "stretch":
      anim.squashTargetX = 0.92;
      anim.squashTargetY = 1.08;
      break;
    case "yawn":
      anim.squashTargetX = 1.05;
      anim.squashTargetY = 0.95;
      break;
    case "walk":
      startWalk(anim, semantic);
      break;
    case "playDuck":
      startToy(anim, "duck", emit);
      break;
    case "playDice":
      startToy(anim, "dice", emit);
      break;
    case "drinkCoffee":
      startToy(anim, "coffee", emit);
      break;
    case "playBug":
      startToy(anim, "bug", emit);
      break;
    case "readScroll":
      startToy(anim, "scroll", emit);
      break;
    case "doze":
      say(anim, pickFrom(STATUS_POOLS.doze), -30);
      break;
    case "nodOff":
      anim.lidBase = Math.max(anim.lidBase, 0.3);
      if (Math.random() < 0.5) say(anim, pickFrom(STATUS_POOLS.nod_off));
      break;
    case "confidentPose":
      setStatus(anim, "( ᵔ ᴥ ᵔ )", 100);
      anim.eyeStyle = "squint";
      anim.eyeStyleTimer = 90;
      break;
    case "fidget":
      anim.squashTargetX = 0.88 + Math.random() * 0.24;
      anim.squashTargetY = 1.12 - Math.random() * 0.24;
      say(anim, pickFrom(STATUS_POOLS.fidget), -40);
      break;
    case "wave":
      anim.earState = 1;
      if (Math.random() < 0.35) {
        anim.eyeStyle = "wink";
        anim.eyeStyleTimer = 70;
      }
      say(anim, pickFrom(STATUS_POOLS.wave), -20);
      break;
    case "spin":
      spawnRainbowSparks(anim, 7);
      anim.squashTargetX = 1.1;
      anim.squashTargetY = 0.92;
      say(anim, pickFrom(STATUS_POOLS.spin), -40);
      break;
    case "type_code":
      say(anim, pickFrom(STATUS_POOLS.type_code), 50);
      break;
    case "scratch":
      anim.earState = -0.3;
      say(anim, pickFrom(STATUS_POOLS.scratch), -30);
      break;
    case "peekAround":
      anim.earState = 0.5;
      break;
    case "sniff":
      anim.earState = 0.3;
      break;
    case "sigh":
      anim.sighCooldown = 700;
      break;
    case "earTwitch":
      anim.earTwitchTimer = 12;
      anim.earTwitchSide = Math.random() < 0.5 ? -1 : 1;
      break;
    case "headTiltHold":
      anim.earTwitchSide = Math.random() < 0.5 ? -1 : 1;
      anim.earState = 0.6;
      if (Math.random() < 0.4)
        spawnFloatingEmoji(
          anim,
          "?",
          CANVAS_CENTER_X + anim.walkOffsetX + 10,
          CANVAS_CENTER_Y - 26,
        );
      break;
    case "groom":
      say(anim, pickFrom(STATUS_POOLS.groom), -20);
      break;
    case "wiggle":
      if (Math.random() < 0.5) say(anim, pickFrom(STATUS_POOLS.wiggle), -40);
      anim.earState = 1;
      break;
    case "zoomies":
      startZoomies(anim, semantic);
      break;
    case "shiver":
      anim.shiverTimer = 26;
      break;
    case "daydream":
      anim.gazeSettleFrames = 0;
      break;
    case "peekCamera":
      break;
    case "shakeOff":
      say(anim, pickFrom(STATUS_POOLS.shakeoff), -40);
      spawnGroundEffect(
        anim,
        "dust",
        CANVAS_CENTER_X + anim.walkOffsetX - 6,
        CANVAS_CENTER_Y + 11,
      );
      spawnGroundEffect(
        anim,
        "dust",
        CANVAS_CENTER_X + anim.walkOffsetX + 6,
        CANVAS_CENTER_Y + 11,
      );
      break;
    default:
      break;
  }
}

function updateIdleActionFrame(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
): void {
  const elapsed = anim.idleActionTotal - anim.idleActionTimer;
  const cx = CANVAS_CENTER_X + anim.walkOffsetX;

  switch (anim.idleAction) {
    case "fidget":
      if (anim.frame % 18 === 0) {
        anim.squashTargetX = 0.93 + Math.random() * 0.12;
        anim.squashTargetY = 1.07 - Math.random() * 0.12;
      }
      break;
    case "wave": {
      const wm = Math.sin(anim.frame * 0.28) * 0.08;
      anim.squashTargetX = 0.94 + wm;
      anim.squashTargetY = 1.06 - wm;
      break;
    }
    case "spin":
      anim.squashTargetX = 1.08 + Math.sin(anim.frame * 0.45) * 0.08;
      anim.squashTargetY = 0.93 - Math.sin(anim.frame * 0.45) * 0.08;
      if (anim.frame % 5 === 0) {
        spawnGroundEffect(
          anim,
          "dust",
          cx + (Math.random() - 0.5) * 24,
          CANVAS_CENTER_Y + 12,
        );
      }
      break;
    case "type_code": {
      const tp = Math.abs(Math.sin(anim.frame * 0.38));
      anim.squashTargetX = 1.02 + tp * 0.05;
      anim.squashTargetY = 0.98 - tp * 0.05;
      break;
    }
    case "scratch":
      anim.headTilt = Math.sin(anim.frame * 0.25) * 0.35;
      if (anim.frame % 8 === 0) {
        anim.squashTargetX = 0.95 + Math.random() * 0.04;
        anim.squashTargetY = 1.05 - Math.random() * 0.04;
      }
      break;
    case "lookAround":
      anim.cursorTargetX = Math.sin(anim.idleActionTimer * 0.15) * 1.5;
      break;
    case "hover":
      if (anim.frame % 40 === 0) {
        anim.floatingEmojis.push({
          emoji: "💕",
          x: cx,
          y: CANVAS_CENTER_Y - 18,
          velocityX: 0,
          velocityY: -0.3,
          life: 1,
        });
      }
      break;
    case "peekAround": {
      const t = anim.idleActionTimer;
      if (t > 60) anim.cursorTargetX = -1.6;
      else if (t > 30) anim.cursorTargetX = 1.6;
      else anim.cursorTargetX += (0 - anim.cursorTargetX) * 0.08;
      break;
    }
    case "sniff": {
      const sniffElapsed = 32 - anim.idleActionTimer;
      const phase = Math.sin((sniffElapsed / 32) * Math.PI * 2.5) * 0.5;
      anim.squashTargetX = 0.97 - phase * 0.03;
      anim.squashTargetY = 1.03 + phase * 0.05;
      anim.eyeLookY += (0.5 - anim.eyeLookY) * 0.12;
      break;
    }
    case "doze":
      if (anim.frame % 90 === 0)
        setStatus(anim, pickFrom(STATUS_POOLS.doze), 100);
      if (semantic.mood.energy > 55 && Math.random() < 0.004) {
        anim.idleAction = "stretch";
        anim.idleActionTimer = 55;
        anim.idleActionTotal = 55;
        anim.lidBase = 0;
        anim.squashTargetX = 0.92;
        anim.squashTargetY = 1.08;
        say(anim, pickFrom(STATUS_POOLS.wake_stretch));
        anim.nextIdleBias = { yawn: 2, shakeOff: 2.5 };
      }
      break;
    case "sigh":
      if (elapsed < 14) {
        anim.squashTargetX = 0.96;
        anim.squashTargetY = 1.06;
      } else if (elapsed === 22) {
        anim.squashTargetX = 1.08;
        anim.squashTargetY = 0.92;
        anim.breathPhase = Math.PI;
        spawnGroundEffect(anim, "dust", cx, CANVAS_CENTER_Y + 12);
        if (Math.random() < 0.6) say(anim, pickFrom(STATUS_POOLS.sigh), -30);
      }
      break;
    case "headTiltHold": {
      const dir = anim.earTwitchSide >= 0 ? 1 : -1;
      const hold = elapsed > 12 && anim.idleActionTimer > 16;
      const target = hold ? dir * 0.55 : 0;
      anim.headTilt += (target - anim.headTilt) * 0.12;
      break;
    }
    case "groom":
      if (anim.frame % 14 === 0) {
        anim.squashTargetX = 0.96 + Math.random() * 0.06;
        anim.squashTargetY = 1.04 - Math.random() * 0.06;
      }
      if (anim.frame % 18 === 0) spawnSparks(anim, 1, "#FFFFFF");
      anim.eyeLookY += (0.4 - anim.eyeLookY) * 0.1;
      break;
    case "wiggle": {
      const w = Math.sin(elapsed * 0.9) * 0.09;
      anim.squashTargetX = 1.02 + w;
      anim.squashTargetY = 0.98 - w;
      if (anim.idleActionTimer === 1 && Math.random() < 0.5) {
        startWalk(anim, semantic);
        anim.idleAction = "walk";
        anim.idleActionTimer = 999;
        anim.idleActionTotal = 999;
        anim.walkSpeed *= 1.4;
      }
      break;
    }
    case "daydream":
      anim.cursorTargetX = -0.75;
      anim.cursorTargetY = -0.65;
      anim.lidBase = Math.max(anim.lidBase, 0.2);
      if (elapsed === 12)
        spawnFloatingEmoji(anim, "💭", cx + 14, CANVAS_CENTER_Y - 22);
      if (elapsed > 12 && elapsed % 55 === 0)
        spawnFloatingEmoji(
          anim,
          pickFrom(["🐟", "⭐", "🍕", "🦋", "🌈"] as const),
          cx + 16,
          CANVAS_CENTER_Y - 26,
        );
      if (elapsed === 80 && Math.random() < 0.5)
        say(anim, pickFrom(STATUS_POOLS.daydream), -20);
      break;
    case "nodOff": {
      const dip = Math.abs(Math.sin(elapsed * 0.09));
      anim.squashTargetY = 1 - dip * 0.07;
      anim.squashTargetX = 1 + dip * 0.05;
      anim.lidClose = Math.max(anim.lidClose, dip * 0.7);
      if (anim.idleActionTimer === 1) {
        anim.idleAction = "doze";
        const dur = getIdleActionDuration("doze") | 0;
        anim.idleActionTimer = dur;
        anim.idleActionTotal = dur;
        say(anim, pickFrom(STATUS_POOLS.doze), -30);
      }
      break;
    }
    case "peekCamera":
      anim.cursorTargetX = 0;
      anim.cursorTargetY = 0.05;
      if (elapsed === 24) {
        if (Math.random() < 0.5)
          say(anim, pickFrom(STATUS_POOLS.peek_camera), -20);
        anim.slowBlinkTimer = 46;
      }
      break;
    case "shakeOff":
      anim.squashTargetX = 1 + Math.sin(elapsed * 1.4) * 0.16;
      anim.squashTargetY = 1 - Math.sin(elapsed * 1.4) * 0.12;
      if (anim.frame % 4 === 0)
        spawnGroundEffect(
          anim,
          "dust",
          cx + (Math.random() - 0.5) * 16,
          CANVAS_CENTER_Y + 11,
        );
      break;
    case "shiver":
      anim.shiverTimer = Math.max(anim.shiverTimer, 2);
      break;
    case "stumble":
      if (elapsed < 10) {
        anim.squashTargetX = 1.28;
        anim.squashTargetY = 0.72;
      } else if (elapsed < 24) {
        anim.squashTargetX = 0.97;
        anim.squashTargetY = 1.03;
      } else if (elapsed % 10 === 0) {
        anim.cursorTargetX = elapsed % 20 === 0 ? 1.3 : -1.3;
        anim.saccadeFrames = 3;
      }
      break;
    default:
      break;
  }
}

export function stepAnimFrame(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
  emit: (e: BuddyEvent) => void,
): void {
  anim.frame++;

  processBeats(anim);

  anim.bobPhase +=
    anim.idleAction === "doze"
      ? 0.03
      : 0.04 + (semantic.mood.energy / 100) * 0.05;
  anim.squashX += (anim.squashTargetX - anim.squashX) * 0.12;
  anim.squashY += (anim.squashTargetY - anim.squashY) * 0.12;
  anim.squashTargetX += (1 - anim.squashTargetX) * 0.04;
  anim.squashTargetY += (1 - anim.squashTargetY) * 0.04;
  if (anim.screenFlash > 0.01) anim.screenFlash *= 0.85;
  else anim.screenFlash = 0;
  if (anim.screenGlitch > 0.01) anim.screenGlitch *= 0.88;
  else anim.screenGlitch = 0;
  if (anim.shakeIntensity > 0.3) anim.shakeIntensity *= 0.82;
  else anim.shakeIntensity = 0;

  if (anim.sighCooldown > 0) anim.sighCooldown--;
  if (anim.rareActionCooldown > 0) anim.rareActionCooldown--;
  if (anim.stumbleCooldown > 0) anim.stumbleCooldown--;
  if (anim.blushTimer > 0) anim.blushTimer--;
  if (anim.earTwitchTimer > 0) anim.earTwitchTimer--;
  if (anim.shiverTimer > 0) anim.shiverTimer--;

  if (anim.combo.displayTimer > 0 && anim.frame % 6 === 0) {
    anim.sparks.push({
      x: CANVAS_CENTER_X + anim.walkOffsetX + (Math.random() - 0.5) * 40,
      y: CANVAS_CENTER_Y + (Math.random() - 0.5) * 20 - 8,
      velocityX: (Math.random() - 0.5) * 1.2,
      velocityY: -0.4 - Math.random() * 1.2,
      life: 1,
      color: `hsl(${anim.combo.rainbowHue},100%,60%)`,
    });
  }

  updateBlink(anim, semantic);
  updateGaze(anim, semantic);
  updateBreath(anim, semantic);

  if (anim.celebrationTimer > 0) anim.celebrationTimer--;
  if (anim.eyeStyleTimer > 0) {
    anim.eyeStyleTimer--;
    if (anim.eyeStyleTimer === 0) anim.eyeStyle = "normal";
  }
  if (anim.combo.displayTimer > 0) {
    anim.combo.displayTimer--;
    anim.combo.rainbowHue = (anim.combo.rainbowHue + 5) % 360;
  }

  const excited =
    anim.moodType === "happy" ||
    anim.moodType === "celebrate" ||
    anim.mouseOnBuddy ||
    anim.hoverGlow > 0.5;
  const scared = semantic.mood.anxiety > 65;
  const pupilTarget = scared ? 0 : excited ? 1 : 0.45;
  anim.pupilDilation += (pupilTarget - anim.pupilDilation) * 0.08;

  anim.heat = Math.max(0, anim.heat - 0.15);
  anim.earAnimProgress += (anim.earState - anim.earAnimProgress) * 0.08;
  const happinessOffset = semantic.mood.happiness < 35 ? -0.22 : 0;
  if (anim.idleAction !== "headTiltHold" && anim.idleAction !== "scratch") {
    anim.headTilt +=
      (anim.cursorTargetX * 0.6 + happinessOffset - anim.headTilt) * 0.08;
  }
  anim.hoverGlow += ((anim.mouseOnBuddy ? 1 : 0) - anim.hoverGlow) * 0.1;
  if (
    semantic.mood.anxiety > 55 &&
    !anim.mouseOnBuddy &&
    anim.idleAction !== "doze" &&
    anim.frame % 80 === 0 &&
    Math.random() < 0.7
  ) {
    anim.cursorTargetX = (Math.random() - 0.5) * 2.8;
    anim.cursorTargetY = (Math.random() - 0.5) * 0.7;
    anim.saccadeFrames = 3;
  }
  if (anim.statusTimer > 0) {
    anim.statusTimer--;
    if (anim.statusTimer === 0) anim.statusTargetOpacity = 0;
  } else if (anim.statusTargetOpacity > 0 && anim.statusText) {
    anim.statusTimer = 180;
  }
  anim.statusOpacity += (anim.statusTargetOpacity - anim.statusOpacity) * 0.07;
  if (anim.statusOpacity < 0.02 && anim.statusTargetOpacity === 0) {
    anim.statusOpacity = 0;
    anim.statusText = "";
  }

  const stage = semantic.progress.stage;
  anim.moodType = semantic.activity.mood;
  anim.levitationOffset = stage >= 5 ? Math.sin(anim.frame * 0.03) * 3 : 0;
  anim.auraPulseIntensity =
    stage >= 5 ? 0.5 + Math.sin(anim.frame * 0.04) * 0.5 : 0;

  anim.stageQuirkTick++;
  if (
    (semantic.activity.animationType === "idle" ||
      anim.idleAction === "doze") &&
    !anim.quirkActive &&
    Math.random() < 0.004 * (1 + semantic.personality.chaos * 0.01)
  ) {
    type StageQuirk = { type: string; duration: number; onStart?: () => void };
    const quirkMap: Partial<Record<number, StageQuirk[]>> = {
      0: [{ type: "rock", duration: 1000 }],
      1: [{ type: "shell_fall", duration: 1500 }],
      2: [{ type: "phase", duration: 1500 }],
      3: [
        {
          type: "mischief",
          duration: 2000,
          onStart: () => {
            setStatus(anim, "hehehe...", 160);
          },
        },
      ],
      4: [
        {
          type: "shadowclone",
          duration: 2000,
          onStart: () => {
            anim.shadowClone = {
              x: CANVAS_CENTER_X - 20 + Math.random() * 40,
              y: CANVAS_CENTER_Y - 5,
              alpha: 0.4,
              life: 0.8,
            };
            setStatus(anim, "shadow clone!", 180);
          },
        },
      ],
      5: [
        {
          type: "meditate",
          duration: 3000,
          onStart: () => {
            setStatus(anim, "om...", 220);
            anim.eyeStyle = "squint";
            anim.eyeStyleTimer = 180;
          },
        },
      ],
    };
    const quirks: StageQuirk[] = quirkMap[stage] ?? [];
    if (quirks.length > 0) {
      const q = quirks[Math.floor(Math.random() * quirks.length)];
      anim.quirkActive = true;
      anim.quirkType = q.type;
      anim.stageQuirkTick = 0;
      anim.quirkEndFrame = anim.frame + Math.round((q.duration / 1000) * 60);
      q.onStart?.();
    }
  }

  if (anim.quirkActive && anim.frame >= anim.quirkEndFrame) {
    anim.quirkActive = false;
    anim.quirkType = "";
  }

  if (anim.shadowClone) {
    anim.shadowClone.life -= 0.015;
    anim.shadowClone.alpha = anim.shadowClone.life;
    if (anim.shadowClone.life <= 0) anim.shadowClone = null;
  }

  updateWalk(anim, semantic);

  if (anim.toyActive) {
    anim.toyAnimPhase += 0.12;
    anim.toyDurationTimer--;
    if (anim.toyDurationTimer <= 0) stopToy(anim, semantic, emit);
  }

  if (anim.mouseProximity > 0.6 && anim.mouseOnBuddy) {
    anim.mouseNearTimer++;
    if (anim.mouseNearTimer > 120) {
      const tx = anim.cursorTargetX * 18;
      const ty = anim.cursorTargetY * 12;
      anim.nuzzleOffsetX += (tx - anim.nuzzleOffsetX) * 0.04;
      anim.nuzzleOffsetY += (ty - anim.nuzzleOffsetY) * 0.04;
      if (
        Math.abs(anim.nuzzleOffsetX - tx) < 1 &&
        anim.mouseNearTimer % 90 === 0
      ) {
        anim.squashTargetX = 1.05;
        anim.squashTargetY = 0.96;
        if (Math.random() < 0.3) spawnSparks(anim, 2, "#F472B6");
        setStatus(anim, "( ˘ ³˘)♥", 90);
        emit({
          type: "semantic_update",
          patch: {
            mood: {
              ...semantic.mood,
              affection: Math.min(100, semantic.mood.affection + 2),
            },
          },
        });
      }
    }
  } else {
    anim.mouseNearTimer = Math.max(0, anim.mouseNearTimer - 2);
    anim.nuzzleOffsetX += (0 - anim.nuzzleOffsetX) * 0.06;
    anim.nuzzleOffsetY += (0 - anim.nuzzleOffsetY) * 0.06;
  }

  if (anim.frame % 30 === 0) updateMoodDrift(anim, semantic, emit);

  if (anim.activeScene) {
    if (anim.activeSceneTimer > 0) {
      anim.activeSceneTimer--;
    } else {
      anim.activeScene = "";
      anim.activeSceneVariant = "";
    }
    if (anim.activeScene) {
      updateSceneAnimation(anim, anim.activeScene, anim.activeSceneVariant);
    }
  }

  if (
    semantic.activity.animationType !== "idle" &&
    anim.idleAction !== "doze"
  ) {
    anim.idleAction = "none";
    return;
  }

  if (
    anim.mouseSpeed > 0.15 &&
    anim.mouseProximity > 0.5 &&
    anim.idleAction === "none"
  ) {
    anim.idleAction = "startled";
    anim.idleActionTimer = 30;
    anim.idleActionTotal = 30;
    anim.squashTargetX = 0.88;
    anim.squashTargetY = 1.12;
    anim.eyeStyle = "wide";
    anim.eyeStyleTimer = 26;
  }
  if (anim.mouseOnBuddy && anim.idleAction === "none") {
    anim.idleAction = "hover";
    anim.idleActionTimer = 999;
    anim.idleActionTotal = 999;
    if (Math.random() < 0.04) spawnSparks(anim, 1);
  }
  if (anim.idleAction === "hover" && !anim.mouseOnBuddy) {
    anim.idleAction = "none";
    anim.idleActionTimer = 0;
  }
  if (
    anim.mouseProximity > 0.5 &&
    anim.mouseProximity < 0.8 &&
    !anim.mouseOnBuddy &&
    anim.idleAction === "none"
  ) {
    anim.idleAction = "curious";
    anim.idleActionTimer = 60;
    anim.idleActionTotal = 60;
    anim.squashTargetX = 0.92;
    anim.squashTargetY = 1.08;
  }
  if (anim.idleAction === "curious" && anim.mouseProximity < 0.2) {
    anim.idleAction = "lookBack";
    anim.idleActionTimer = 40;
    anim.idleActionTotal = 40;
  }

  if (
    semantic.mood.anxiety > 35 &&
    anim.mouseSpeed > 0.1 &&
    anim.mouseProximity > 0.4 &&
    anim.idleAction === "none"
  ) {
    anim.nuzzleOffsetX += (anim.cursorTargetX > 0 ? -1 : 1) * 3;
    anim.squashTargetX = 0.9;
    anim.squashTargetY = 1.1;
    if (Math.random() < 0.12) spawnSparks(anim, 2, "#FF4444");
  }
  if (
    semantic.personality.playfulness > 55 &&
    anim.mouseSpeed > 0.08 &&
    anim.mouseProximity > 0.5
  ) {
    if (Math.random() < 0.03) {
      anim.squashTargetX = 0.93;
      anim.squashTargetY = 1.07;
    }
  }

  updateIdleActionFrame(anim, semantic);

  if (
    anim.idleAction === "none" &&
    anim.mouseProximity < 0.2 &&
    Math.random() < 0.004
  ) {
    startIdleAction(anim, semantic, emit, selectIdleAction(anim, semantic));
  }

  if (anim.idleActionTimer > 0) {
    anim.idleActionTimer--;
    if (
      anim.idleActionTimer <= 0 &&
      !PERSISTENT_TOY_ACTIONS.has(anim.idleAction)
    ) {
      const ended = anim.idleAction;
      const chain = IDLE_CHAIN_BIAS[ended];
      if (chain !== undefined) anim.nextIdleBias = { ...chain };
      anim.idleAction = "none";
    }
  }
}

export function handlePet(
  anim: BuddyAnimState,
  canvasX: number,
  canvasY: number,
  emit: (e: BuddyEvent) => void,
  stage = 0,
  semantic?: BuddySemanticState,
): void {
  const buddyX = CANVAS_CENTER_X + anim.walkOffsetX;
  const [spriteW] = STAGE_SIZES[stage] ?? [28, 18];
  const hitRadius = spriteW / 2 + 4;
  const dist = Math.sqrt(
    (canvasX - buddyX) ** 2 + (canvasY - CANVAS_CENTER_Y) ** 2,
  );
  if (dist > hitRadius) return;

  if (anim.frame - anim.lastPetFrame > 600) anim.petSessionCount = 0;
  anim.lastPetFrame = anim.frame;
  anim.petSessionCount++;
  const s = anim.petSessionCount;
  const anxious = (semantic?.mood.anxiety ?? 0) > 50 || anim.errorStreak >= 3;

  anim.nuzzleOffsetX += clamp((canvasX - buddyX) * 0.15, -3, 3);
  anim.petCount++;
  anim.successStreak++;
  anim.errorStreak = Math.max(0, anim.errorStreak - 1);

  if (anxious && s === 1) {
    anim.squashTargetX = 0.9;
    anim.squashTargetY = 1.1;
    anim.eyeStyle = "wide";
    anim.eyeStyleTimer = 30;
    say(anim, pickFrom(STATUS_POOLS.pet_flinch), -20);
    emit({ type: "petted" });
    return;
  }
  if (s > 14) {
    anim.eyeStyle = "shifty";
    anim.eyeStyleTimer = 80;
    anim.squashTargetX = 0.95;
    anim.squashTargetY = 1.05;
    say(anim, pickFrom(STATUS_POOLS.pet_max), -20);
    if (s === 15)
      spawnFloatingEmoji(anim, "💢", buddyX + 10, CANVAS_CENTER_Y - 20);
    emit({ type: "petted" });
    return;
  }

  anim.squashTargetX = 1.08;
  anim.squashTargetY = 0.94;
  spawnSparks(anim, Math.min(2 + s, 7), "#F472B6");
  if (s >= 4) anim.blushTimer = Math.max(anim.blushTimer, 120);
  if (s >= 8 && anim.slowBlinkTimer === 0 && Math.random() < 0.3)
    anim.slowBlinkTimer = 50;

  if (s >= 8) {
    anim.eyeStyle = "heart";
    anim.eyeStyleTimer = 140;
    say(anim, pickFrom(STATUS_POOLS.pet_love), -20);
  } else if (s >= 3) {
    anim.eyeStyle = s % 2 === 0 ? "squint" : "uwu";
    anim.eyeStyleTimer = 120;
    say(anim, pickFrom(STATUS_POOLS.pet_warm), -30);
  } else {
    setStatus(anim, "*happy*", 75);
    if (anim.petCount % 10 === 0) {
      anim.eyeStyle = "uwu";
      anim.eyeStyleTimer = 200;
    }
  }

  emit({ type: "petted" });
}
