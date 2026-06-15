import type {
  BuddyCareAction,
  BuddyScenePose,
  BuddySpeechStyle,
} from "./types";

export interface BuddyCareActivityDef {
  spot: { x: number; y: number };
  depthScale: number;
  pose: BuddyScenePose;
  performMs: number;
  startLines: readonly ((name: string) => string)[];
  midLines: readonly ((name: string) => string)[];
  midStyle: BuddySpeechStyle;
  finishLines: readonly ((name: string) => string)[];
}

export interface BuddyCareActivity {
  action: BuddyCareAction;
  toy?: string;
  startedAtMs: number;
  travelMs: number;
  performMs: number;
}

export const BUDDY_CARE_ACTIVITY_DEFS: Record<
  BuddyCareAction,
  BuddyCareActivityDef
> = {
  feed: {
    spot: { x: 38, y: 78 },
    depthScale: 0.98,
    pose: "bounce",
    performMs: 6_800,
    startLines: [
      (name) => `${name} sprints to the noodle bowl. Priorities!`,
      (name) => `${name} smelled dinner from across the meadow.`,
      (name) => `Snack time! ${name} is already drooling a little.`,
    ],
    midLines: [
      (name) => `${name}: nom nom nom…`,
      (name) => `${name} makes happy chewing noises.`,
      (name) => `${name} whispers: the broth… the BROTH…`,
    ],
    midStyle: "whisper",
    finishLines: [
      (name) => `${name} licks the bowl clean. Five stars.`,
      (name) => `Burp. ${name} regrets nothing.`,
      (name) => `${name} pats a happy round belly.`,
    ],
  },
  play: {
    spot: { x: 47, y: 80 },
    depthScale: 1.02,
    pose: "pounce",
    performMs: 7_600,
    startLines: [
      (name) => `${name} crouches... wiggles... GAME ON.`,
      (name) => `${name} drops everything for playtime.`,
      (name) => `Play mode engaged. ${name} is unstoppable now.`,
    ],
    midLines: [
      (name) => `${name}: wiggle… wiggle… POUNCE.`,
      (name) => `${name} stalks the ball. It suspects nothing.`,
      (name) => `${name} plots round two. Sneakier this time.`,
    ],
    midStyle: "whisper",
    finishLines: [
      (name) => `${name} flops over, victorious and breathless.`,
      (name) => `Final score: ${name} 1, ball 0.`,
      (name) => `${name} hides the toy for next time. Sneaky.`,
    ],
  },
  clean: {
    spot: { x: 36, y: 82 },
    depthScale: 1.04,
    pose: "spin",
    performMs: 7_000,
    startLines: [
      (name) => `${name} tiptoes into the pond shallows. Bath time!`,
      (name) => `${name} eyes the water suspiciously, then dives in.`,
      (name) => `Operation Sparkle: ${name} reporting for scrub duty.`,
    ],
    midLines: [
      (name) => `${name}: scrub scrub scrub, behind the ears too.`,
      (name) => `${name} acquires a bubble beard.`,
      (name) => `${name}'s sparkle is coming back…`,
    ],
    midStyle: "whisper",
    finishLines: [
      (name) => `${name} shakes dry in a cloud of sparkles. So fluffy!`,
      (name) => `Squeaky clean! ${name} gleams like a river stone.`,
      (name) => `${name} admires the reflection. Magnificent.`,
    ],
  },
  sleep: {
    spot: { x: 34, y: 78 },
    depthScale: 0.98,
    pose: "sleepy",
    performMs: 8_400,
    startLines: [
      (name) => `${name} pads to the great tree and curls up tight.`,
      (name) => `Yawn... ${name} found the softest moss patch.`,
      (name) => `${name} tucks in under the leaves. Shhh.`,
    ],
    midLines: [
      (name) => `${name}: zzz… giant acorns…`,
      (name) => `${name}: zzz… flying over soft hills…`,
      (name) => `${name}: zzz… Brick swimming through stars…`,
    ],
    midStyle: "think",
    finishLines: [
      (name) => `${name} wakes up recharged and ready!`,
      (name) => `*stretch* ${name} dreamed of giant acorns.`,
      (name) => `${name} blinks awake, batteries full.`,
    ],
  },
  pet: {
    spot: { x: 50, y: 78 },
    depthScale: 1,
    pose: "bounce",
    performMs: 5_400,
    startLines: [
      (name) => `${name} leans into the head pats. Bliss.`,
      (name) => `${name} melts. Affection levels rising.`,
      (name) => `Pats detected! ${name} wiggles closer.`,
    ],
    midLines: [
      (name) => `${name}: purrrr… more please.`,
      (name) => `${name} is melting. Officially melting.`,
      (name) => `${name} found the best spot. Right there.`,
    ],
    midStyle: "whisper",
    finishLines: [
      (name) => `${name} glows with cozy warmth.`,
      (name) => `${name} files this moment under "best ever".`,
      (name) => `Heart full, ${name} does a tiny happy spin.`,
    ],
  },
};

export function careActorIntentKind(action: BuddyCareAction): string {
  return `care_${action}`;
}

export function careActivityTotalMs(activity: BuddyCareActivity): number {
  return Math.max(0, activity.travelMs) + Math.max(0, activity.performMs);
}

export function pickCareLine(
  lines: readonly ((name: string) => string)[],
  name: string,
): string {
  const line = lines[Math.floor(Math.random() * lines.length)] ?? lines[0];
  return line(name);
}
