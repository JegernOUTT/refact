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
  TOY_DEFS,
  TOY_EMOJI,
  PERSISTENT_TOY_ACTIONS,
} from "../constants";
import type {
  BuddyAnimState,
  BuddySemanticState,
  BuddyEvent,
  IdleActionType,
  ToyType,
  ToyDef,
  SignalDef,
} from "../types";

function selectIdleAction(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
): IdleActionType {
  const m = semantic.mood;
  const p = semantic.personality;
  const candidates = (
    [
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
      { action: "doze", weight: m.energy < 20 ? 25 : 0 },
      {
        action: "confidentPose" as IdleActionType,
        weight: p.confidence > 40 ? p.confidence * 0.09 : 0,
      },
    ] as { action: IdleActionType; weight: number }[]
  ).filter((c) => c.weight > 0.4);

  const total = candidates.reduce((s, c) => s + c.weight, 0);
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
    doze: 250,
    confidentPose: 90,
  };
  return durations[action] ?? 60;
}

function startWalk(anim: BuddyAnimState, semantic: BuddySemanticState): void {
  anim.walking = true;
  const range = 20 + semantic.mood.boredom * 0.3;
  anim.walkTargetX = (Math.random() - 0.5) * range * 2;
  anim.walkDirection = Math.sign(anim.walkTargetX - anim.walkOffsetX) || 1;
  anim.walkSpeed = 0.35 + (semantic.mood.energy / 100) * 0.45;
  anim.walkPhase = 0;
}

function stopWalk(anim: BuddyAnimState): void {
  anim.walking = false;
  anim.idleAction = "none";
  anim.idleActionTimer = 0;
}

function startToy(
  anim: BuddyAnimState,
  toyType: ToyType,
  emit: (e: BuddyEvent) => void,
): void {
  const def: ToyDef | undefined = TOY_DEFS[toyType];
  if (def === undefined) return;
  anim.toyActive = true;
  anim.toyType = toyType;
  anim.toyAnimPhase = 0;
  anim.toyDurationTimer = 140 + Math.random() * 40;
  anim.statusText = def.statusMessage;
  anim.statusTargetOpacity = 1;
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

function updateWalk(anim: BuddyAnimState, semantic: BuddySemanticState): void {
  if (!anim.walking) {
    anim.walkOffsetX *= 0.93;
    if (Math.abs(anim.walkOffsetX) < 0.4) anim.walkOffsetX = 0;
    return;
  }
  anim.walkOffsetX += anim.walkDirection * anim.walkSpeed;
  anim.walkPhase += 0.13 + anim.walkSpeed * 0.04;
  if (anim.frame % 5 === 0)
    spawnGroundEffect(
      anim,
      "dust",
      CANVAS_CENTER_X + anim.walkOffsetX + anim.walkDirection * 6,
      CANVAS_CENTER_Y + 13,
    );
  if (Math.abs(anim.walkOffsetX - anim.walkTargetX) < 1.5) {
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
    patch.energy = Math.min(100, m.energy + 0.018);
  } else {
    patch.boredom = Math.max(0, m.boredom - 0.4);
  }
  patch.anxiety = Math.max(0, m.anxiety - (0.05 + p.resilience * 0.001));
  patch.affection = Math.max(0, m.affection - 0.04);
  if (m.happiness > 58) patch.happiness = Math.max(58, m.happiness - 0.012);
  else if (m.happiness < 48)
    patch.happiness = Math.min(48, m.happiness + 0.012);

  if (anim.mouseProximity > 0.6) {
    emit({
      type: "semantic_update",
      patch: {
        personality: { ...p, clinginess: Math.min(100, p.clinginess + 0.006) },
      },
    });
  }

  emit({ type: "semantic_update", patch: { mood: { ...m, ...patch } } });
}

export function triggerSignalAnimation(
  anim: BuddyAnimState,
  signalType: string,
  emit: (e: BuddyEvent) => void,
): void {
  const def: SignalDef | undefined = SIGNALS[signalType];
  if (def === undefined) return;

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

  const now = Date.now();
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
    anim.squashTargetX = 1.4;
    anim.squashTargetY = 0.6;
    anim.screenFlash = 0.5;
    spawnAfterimage(anim);
    spawnAfterimage(anim);
  }

  if (anim.errorStreak >= 5) {
    anim.eyeStyle = "X";
    anim.eyeStyleTimer = 240;
  } else if (anim.errorStreak >= 3) {
    anim.eyeStyle = "spiral";
    anim.eyeStyleTimer = 300;
  } else if (signalType === "task_failed") {
    anim.eyeStyle = "teary";
    anim.eyeStyleTimer = 200;
  } else if (signalType === "connection_lost" || signalType === "balance_low") {
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

  anim.statusText =
    def.statusTexts[Math.floor(Math.random() * def.statusTexts.length)];
  anim.statusTargetOpacity = 1;

  const cx = CANVAS_CENTER_X + anim.walkOffsetX;
  switch (def.animationType) {
    case "celebrate":
      anim.celebrationTimer = 120;
      spawnSparks(anim, 18);
      spawnFloatingEmoji(anim, def.icon, undefined, undefined, 3);
      anim.squashTargetX = 1.3;
      anim.squashTargetY = 0.7;
      anim.screenFlash = 0.6;
      spawnGroundEffect(anim, "impact", cx, CANVAS_CENTER_Y + 12);
      spawnGroundEffect(anim, "dust", cx - 8, CANVAS_CENTER_Y + 10);
      spawnGroundEffect(anim, "dust", cx + 8, CANVAS_CENTER_Y + 10);
      spawnSpeedLines(anim, 6, 0, -1);
      spawnAfterimage(anim);
      break;
    case "shake":
      anim.shakeIntensity = 7;
      spawnFloatingEmoji(anim, def.icon, cx, CANVAS_CENTER_Y - 24);
      anim.screenGlitch = 0.8;
      anim.squashTargetX = 0.85;
      anim.squashTargetY = 1.15;
      spawnGroundEffect(anim, "crack", cx, CANVAS_CENTER_Y + 12);
      spawnSpeedLines(anim, 4, Math.random() * 6.28, 0);
      break;
    case "eat":
      spawnFloatingEmoji(anim, "🍕", cx + 16, CANVAS_CENTER_Y - 4);
      spawnFloatingEmoji(anim, "🍪", cx - 12, CANVAS_CENTER_Y - 8);
      spawnFloatingEmoji(anim, def.icon);
      anim.squashTargetX = 1.15;
      anim.squashTargetY = 0.85;
      break;
    case "sleep":
      anim.squashTargetX = 1.05;
      anim.squashTargetY = 0.95;
      break;
    case "think":
      spawnFloatingEmoji(anim, def.icon, cx - 16, CANVAS_CENTER_Y - 28);
      anim.squashTargetX = 0.95;
      anim.squashTargetY = 1.05;
      break;
    case "absorb":
      spawnOrbitingOrb(anim, def.icon, 4);
      spawnSparks(anim, 6);
      anim.screenFlash = 0.3;
      anim.squashTargetX = 0.9;
      anim.squashTargetY = 1.1;
      spawnAfterimage(anim);
      break;
    case "work":
      spawnFloatingEmoji(anim, def.icon, undefined, undefined, 2);
      spawnOrbitingOrb(anim, "⚙️", 3);
      spawnSpeedLines(anim, 3, 0, -0.5);
      anim.squashTargetX = 1.08;
      anim.squashTargetY = 0.92;
      break;
    case "perk":
      spawnFloatingEmoji(anim, def.icon, undefined, undefined, 2);
      spawnSparks(anim, 5);
      anim.squashTargetX = 0.85;
      anim.squashTargetY = 1.2;
      anim.screenFlash = 0.15;
      spawnGroundEffect(anim, "dust", cx, CANVAS_CENTER_Y + 12);
      spawnAfterimage(anim);
      break;
  }
}

export function stepAnimFrame(
  anim: BuddyAnimState,
  semantic: BuddySemanticState,
  emit: (e: BuddyEvent) => void,
): void {
  anim.frame++;

  anim.blinkTick++;
  if (anim.blinkTick >= anim.nextBlinkAt && !anim.blinking) {
    anim.blinking = true;
    anim.blinkTick = 0;
    anim.blinkFrames = 8;
  }
  if (anim.blinking) {
    anim.blinkFrames--;
    if (anim.blinkFrames <= 0) {
      anim.blinking = false;
      anim.nextBlinkAt = 80 + Math.random() * 180;
    }
  }

  if (anim.celebrationTimer > 0) anim.celebrationTimer--;
  if (anim.eyeStyleTimer > 0) {
    anim.eyeStyleTimer--;
    if (anim.eyeStyleTimer === 0) anim.eyeStyle = "normal";
  }
  if (anim.combo.displayTimer > 0) {
    anim.combo.displayTimer--;
    anim.combo.rainbowHue = (anim.combo.rainbowHue + 5) % 360;
  }

  const trackSpeed = getCursorTrackSpeed(anim, semantic);
  anim.eyeLookX += (anim.cursorTargetX - anim.eyeLookX) * trackSpeed;
  anim.eyeLookY += (anim.cursorTargetY - anim.eyeLookY) * trackSpeed;
  if (Math.random() < 0.004) {
    anim.cursorTargetX = (Math.random() - 0.5) * 2;
    anim.cursorTargetY = (Math.random() - 0.5) * 2;
  }

  anim.heat = Math.max(0, anim.heat - 0.15);
  anim.earAnimProgress += (anim.earState - anim.earAnimProgress) * 0.08;
  anim.headTilt += (anim.cursorTargetX * 0.6 - anim.headTilt) * 0.08;
  anim.hoverGlow += ((anim.mouseOnBuddy ? 1 : 0) - anim.hoverGlow) * 0.1;
  anim.breathScale = Math.sin(anim.frame * 0.04) * 0.008;
  anim.statusOpacity += (anim.statusTargetOpacity - anim.statusOpacity) * 0.07;

  const stage = semantic.progress.stage;
  anim.levitationOffset = stage >= 5 ? Math.sin(anim.frame * 0.03) * 3 : 0;
  anim.auraPulseIntensity =
    stage >= 5 ? 0.5 + Math.sin(anim.frame * 0.04) * 0.5 : 0;

  anim.stageQuirkTick++;
  if (
    (semantic.activity.animationType === "idle" ||
      anim.idleAction === "doze") &&
    !anim.quirkActive &&
    Math.random() < 0.004
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
            anim.statusText = "hehehe...";
            anim.statusTargetOpacity = 1;
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
            anim.statusText = "shadow clone!";
            anim.statusTargetOpacity = 1;
          },
        },
      ],
      5: [
        {
          type: "meditate",
          duration: 3000,
          onStart: () => {
            anim.statusText = "om...";
            anim.statusTargetOpacity = 1;
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
      q.onStart?.();
      setTimeout(() => {
        anim.quirkActive = false;
        anim.quirkType = "";
      }, q.duration);
    }
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
        anim.squashTargetX = 1.1;
        anim.squashTargetY = 0.9;
        if (Math.random() < 0.3) spawnSparks(anim, 2, "#F472B6");
        anim.statusText = "( ˘ ³˘)♥";
        anim.statusTargetOpacity = 1;
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
    anim.squashTargetX = 0.8;
    anim.squashTargetY = 1.25;
  }
  if (anim.mouseOnBuddy && anim.idleAction === "none") {
    anim.idleAction = "hover";
    anim.idleActionTimer = 999;
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
    anim.squashTargetX = 0.92;
    anim.squashTargetY = 1.08;
  }
  if (anim.idleAction === "curious" && anim.mouseProximity < 0.2) {
    anim.idleAction = "lookBack";
    anim.idleActionTimer = 40;
  }

  if (
    semantic.mood.anxiety > 35 &&
    anim.mouseSpeed > 0.1 &&
    anim.mouseProximity > 0.4 &&
    anim.idleAction === "none"
  ) {
    anim.nuzzleOffsetX += (anim.cursorTargetX > 0 ? -1 : 1) * 3;
    anim.squashTargetX = 0.82;
    anim.squashTargetY = 1.2;
    if (Math.random() < 0.12) spawnSparks(anim, 2, "#FF4444");
  }
  if (
    semantic.personality.playfulness > 55 &&
    anim.mouseSpeed > 0.08 &&
    anim.mouseProximity > 0.5
  ) {
    if (Math.random() < 0.03) {
      anim.squashTargetX = 0.88;
      anim.squashTargetY = 1.18;
    }
  }

  if (
    anim.idleAction === "none" &&
    anim.mouseProximity < 0.2 &&
    Math.random() < 0.004
  ) {
    const action = selectIdleAction(anim, semantic);
    anim.idleAction = action;
    anim.idleActionTimer = getIdleActionDuration(action) | 0;

    if (action === "stretch") {
      anim.squashTargetX = 0.85;
      anim.squashTargetY = 1.15;
    }
    if (action === "yawn") {
      anim.squashTargetX = 1.05;
      anim.squashTargetY = 0.95;
    }
    if (action === "walk") startWalk(anim, semantic);
    if (action === "playDuck") startToy(anim, "duck", emit);
    if (action === "playDice") startToy(anim, "dice", emit);
    if (action === "drinkCoffee") startToy(anim, "coffee", emit);
    if (action === "playBug") startToy(anim, "bug", emit);
    if (action === "readScroll") startToy(anim, "scroll", emit);
    if (action === "doze") {
      anim.statusText = "zzz...";
      anim.statusTargetOpacity = 1;
    }
    if (action === "confidentPose") {
      anim.statusText = "( ᵔ ᴥ ᵔ )";
      anim.statusTargetOpacity = 1;
      anim.eyeStyle = "squint";
      anim.eyeStyleTimer = 90;
    }
    if (action === "fidget") {
      void 0;
    }
  }

  if (anim.idleActionTimer > 0) {
    anim.idleActionTimer--;
    if (
      anim.idleActionTimer <= 0 &&
      !PERSISTENT_TOY_ACTIONS.has(anim.idleAction)
    ) {
      anim.idleAction = "none";
    }
  }
}

export function handlePet(
  anim: BuddyAnimState,
  canvasX: number,
  canvasY: number,
  emit: (e: BuddyEvent) => void,
): void {
  const buddyX = CANVAS_CENTER_X + anim.walkOffsetX;
  const dist = Math.sqrt(
    (canvasX - buddyX) ** 2 + (canvasY - CANVAS_CENTER_Y) ** 2,
  );
  if (dist > 20) return;

  anim.squashTargetX = 1.2;
  anim.squashTargetY = 0.8;
  spawnSparks(anim, 4, "#F472B6");
  anim.petCount++;
  anim.successStreak++;
  anim.errorStreak = Math.max(0, anim.errorStreak - 1);

  if (anim.petCount % 10 === 0) {
    anim.statusText = "uwu";
    anim.eyeStyle = "uwu";
    anim.eyeStyleTimer = 240;
  } else if (anim.petCount % 5 === 0) {
    anim.statusText = "That tickles!";
    anim.eyeStyle = "squint";
    anim.eyeStyleTimer = 150;
  } else if (anim.petCount % 3 === 0) {
    anim.statusText = "Hehe~";
    anim.eyeStyle = "heart";
    anim.eyeStyleTimer = 120;
  } else {
    anim.statusText = "*happy*";
  }
  anim.statusTargetOpacity = 1;

  emit({ type: "semantic_update", patch: {} });
}
