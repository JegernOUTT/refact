import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import classNames from "classnames";
import { BuddyCharacter } from "./BuddyCharacter";
import type {
  BuddyCareAction,
  BuddyControl,
  BuddyEvent,
  BuddyPage,
  BuddyPetState,
  BuddyPulse,
  BuddyQuest,
  BuddyRuntimeEvent,
  BuddyScenePose,
  BuddySemanticState,
  BuddyShowcaseKind,
  BuddyShowcaseRun,
  Palette,
  Stage,
} from "./types";
import {
  buildBuddyWorldState,
  type BuddyWorldObject,
  type BuddyWorldState,
  type BuddyWorldTone,
} from "./buddyWorldModel";
import {
  advanceBuddyShowcasePhase,
  BUDDY_SHOWCASE_IDLE_COOLDOWN_MS,
  BUDDY_SHOWCASE_INITIAL_GRACE_MS,
  BUDDY_SHOWCASE_PHASE_DURATIONS_MS,
  BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS,
  createBuddyShowcaseRun,
  hasBuddyShowcaseRuntimeTrigger,
  type BuddyShowcaseTargetCandidate,
} from "./buddyShowcase";
import { drawShowcaseEvent } from "./buddyShowcaseDraw";
import styles from "./BuddyWorld.module.css";

interface BuddyWorldProps {
  palette: Palette;
  stage: Stage;
  state: BuddySemanticState;
  pulse: BuddyPulse | null | undefined;
  pet: BuddyPetState | undefined;
  nowPlaying: BuddyRuntimeEvent | null;
  activeQuest: BuddyQuest | null;
  activeSpeech: {
    text: string;
    controls: BuddyControl[];
    chat_id?: string;
  } | null;
  setupNeeded: boolean;
  compact?: boolean;
  homeDoorDisabled?: boolean;
  onCanvasEvent: (event: BuddyEvent) => void;
  onCare: (action: BuddyCareAction, toy?: string) => void;
  onOpenPage: (page: BuddyPage) => void;
  onRunMode: (mode: string) => void;
  onDismissSetup: () => void;
  onSpeechControl: (control: BuddyControl) => void;
  now?: Date;
}

const TONE_CLASS: Record<BuddyWorldTone, string> = {
  good: styles.toneGood,
  neutral: styles.toneNeutral,
  warning: styles.toneWarning,
  danger: styles.toneDanger,
};

const SETUP_MODE_ACTIONS = [
  { mode: "setup", label: "Warm up" },
  { mode: "setup_mcp", label: "Link MCP" },
  { mode: "setup_skills", label: "Teach skills" },
] as const;

const HOME_HOTSPOT = { x: 8.5, y: 67 } as const;
const BUDDY_CENTER_X = 50;
const BUDDY_MIN_X = 33;
const BUDDY_MAX_X = 67;
const MAX_RUNTIME_SHOWCASE_EVENT_IDS = 16;

const RANDOM_IDLE_REACTIONS = [
  "Buddy does a tiny spin.",
  "Buddy watches the garden for a moment.",
  "Buddy checks the breeze and grins.",
  "Buddy makes a small happy bounce.",
  "Buddy pauses to inspect a sparkle.",
] as const;

const RANDOM_POSES = [
  "idle",
  "spin",
  "bounce",
  "look",
] as const satisfies readonly BuddyScenePose[];

type BuddyRandomPose = (typeof RANDOM_POSES)[number];

interface BuddyWaypoint {
  id: string;
  x: number;
  y: number;
  label: string;
  reaction: string;
}

function pctX(width: number, value: number): number {
  return (width * value) / 100;
}

function pctY(height: number, value: number): number {
  return (height * value) / 100;
}

function toneColor(tone: BuddyWorldTone): string {
  switch (tone) {
    case "good":
      return "#22C55E";
    case "warning":
      return "#F59E0B";
    case "danger":
      return "#EF4444";
    case "neutral":
      return "#60A5FA";
  }
}

function fillPixelRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  color: string,
): void {
  ctx.fillStyle = color;
  ctx.fillRect(
    Math.round(x),
    Math.round(y),
    Math.round(width),
    Math.round(height),
  );
}

function drawPixelText(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  color: string,
  align: CanvasTextAlign = "center",
): void {
  ctx.save();
  ctx.font = "10px monospace";
  ctx.textAlign = align;
  ctx.textBaseline = "middle";
  ctx.fillStyle = color;
  ctx.fillText(text, x, y);
  ctx.restore();
}

function drawCloud(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  scale: number,
  color: string,
): void {
  fillPixelRect(ctx, x, y + 8 * scale, 34 * scale, 10 * scale, color);
  fillPixelRect(
    ctx,
    x + 6 * scale,
    y + 2 * scale,
    10 * scale,
    8 * scale,
    color,
  );
  fillPixelRect(ctx, x + 16 * scale, y, 12 * scale, 10 * scale, color);
  fillPixelRect(
    ctx,
    x + 28 * scale,
    y + 5 * scale,
    9 * scale,
    9 * scale,
    color,
  );
}

function drawCelestial(
  ctx: CanvasRenderingContext2D,
  world: BuddyWorldState,
  frame: number,
  width: number,
  height: number,
): void {
  const x = pctX(width, world.celestialX);
  const isNight = world.phase === "night";
  const rawY = pctY(height, world.celestialY) + Math.sin(frame / 34) * 2;
  const y = Math.max(isNight ? 38 : 50, rawY);
  const color = isNight ? "#E0E7FF" : "#FBBF24";
  const glow = isNight ? "rgba(129,140,248,0.24)" : "rgba(251,191,36,0.26)";

  ctx.fillStyle = glow;
  ctx.beginPath();
  ctx.arc(x, y, isNight ? 34 : 42, 0, Math.PI * 2);
  ctx.fill();

  fillPixelRect(ctx, x - 13, y - 13, 26, 26, color);
  fillPixelRect(ctx, x - 18, y - 8, 36, 16, color);
  fillPixelRect(ctx, x - 8, y - 18, 16, 36, color);

  if (isNight) {
    fillPixelRect(ctx, x + 4, y - 13, 14, 26, "#4C1D95");
  } else {
    fillPixelRect(ctx, x - 2, y - 32, 4, 8, "#F59E0B");
    fillPixelRect(ctx, x - 2, y + 24, 4, 8, "#F59E0B");
    fillPixelRect(ctx, x - 32, y - 2, 8, 4, "#F59E0B");
    fillPixelRect(ctx, x + 24, y - 2, 8, 4, "#F59E0B");
  }
}

function drawDistantHills(
  ctx: CanvasRenderingContext2D,
  world: BuddyWorldState,
  frame: number,
  width: number,
  height: number,
): void {
  const farY = height * 0.62;
  const nearY = height * 0.69;
  const farColor = world.phase === "night" ? "#1E3A5F" : "#2F855A";
  const nearColor = world.phase === "night" ? "#155E49" : "#166534";

  ctx.save();
  ctx.fillStyle = `${farColor}66`;
  ctx.beginPath();
  ctx.moveTo(0, farY + 18);
  for (let x = 0; x <= width; x += 20) {
    const y = farY + Math.sin(x / 56 + frame / 210) * 8;
    ctx.lineTo(x, y);
  }
  ctx.lineTo(width, height);
  ctx.lineTo(0, height);
  ctx.closePath();
  ctx.fill();

  ctx.fillStyle = `${nearColor}88`;
  ctx.beginPath();
  ctx.moveTo(0, nearY + 16);
  for (let x = 0; x <= width; x += 16) {
    const y = nearY + Math.sin(x / 42 + frame / 180) * 6 + Math.sin(x / 19) * 2;
    ctx.lineTo(x, y);
  }
  ctx.lineTo(width, height);
  ctx.lineTo(0, height);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function drawWeather(
  ctx: CanvasRenderingContext2D,
  world: BuddyWorldState,
  frame: number,
  width: number,
  height: number,
): void {
  const x = pctX(width, world.weatherX);
  const y = pctY(height, world.weatherY);

  if (world.weather === "storm") {
    drawCloud(ctx, x - 40, y - 15, 1.5, "#475569");
    fillPixelRect(ctx, x + 4, y + 26, 8, 22, "#FACC15");
    fillPixelRect(ctx, x - 2, y + 40, 8, 16, "#FACC15");
    for (let i = 0; i < 16; i += 1) {
      const rx = x - 60 + ((i * 17 + frame * 2) % 130);
      const ry = y + 18 + ((i * 11 + frame) % 64);
      fillPixelRect(ctx, rx, ry, 2, 8, "rgba(125,211,252,0.75)");
    }
    return;
  }

  if (world.weather === "rain") {
    drawCloud(ctx, x - 45, y - 10, 1.45, "#94A3B8");
    for (let i = 0; i < 18; i += 1) {
      const rx = x - 54 + ((i * 13 + frame) % 112);
      const ry = y + 18 + ((i * 19 + frame * 2) % 72);
      fillPixelRect(ctx, rx, ry, 2, 7, "rgba(56,189,248,0.72)");
    }
    return;
  }

  if (world.weather === "wind") {
    for (let i = 0; i < 5; i += 1) {
      const wx = x - 70 + ((frame * (1 + i * 0.22) + i * 36) % 150);
      const wy = y + i * 12;
      fillPixelRect(ctx, wx, wy, 36, 2, "rgba(255,255,255,0.52)");
      fillPixelRect(ctx, wx + 28, wy + 3, 18, 2, "rgba(255,255,255,0.38)");
    }
    return;
  }

  if (world.weather === "busy") {
    ctx.strokeStyle = "rgba(96,165,250,0.18)";
    ctx.lineWidth = 1;
    for (let i = 0; i < 3; i += 1) {
      ctx.beginPath();
      ctx.arc(
        x,
        y,
        8 + i * 8 + Math.sin(frame / 22 + i) * 1.2,
        0,
        Math.PI * 1.35,
      );
      ctx.stroke();
    }
    return;
  }

  if (world.weather === "dream") {
    for (let i = 0; i < 4; i += 1) {
      drawPixelText(
        ctx,
        "Z",
        x + i * 20,
        y + Math.sin(frame / 16 + i) * 8,
        "#C4B5FD",
      );
    }
    return;
  }

  if (world.weather === "aurora") {
    for (let i = 0; i < 4; i += 1) {
      ctx.strokeStyle =
        i % 2 === 0 ? "rgba(45,212,191,0.45)" : "rgba(168,85,247,0.45)";
      ctx.lineWidth = 8;
      ctx.beginPath();
      ctx.moveTo(0, y + i * 10);
      ctx.bezierCurveTo(
        width * 0.28,
        y - 28 + i * 8,
        width * 0.6,
        y + 36 - i * 6,
        width,
        y - 8 + i * 8,
      );
      ctx.stroke();
    }
  }
}

function drawGround(
  ctx: CanvasRenderingContext2D,
  frame: number,
  width: number,
  height: number,
): void {
  const baseY = height * 0.745;

  ctx.save();
  ctx.fillStyle = "rgba(22, 101, 52, 0.46)";
  ctx.beginPath();
  ctx.moveTo(0, baseY + 10);
  for (let x = 0; x <= width; x += 18) {
    const hill = Math.sin(x / 48 + frame / 150) * 7 + Math.sin(x / 19) * 2;
    ctx.lineTo(x, baseY + hill);
  }
  ctx.lineTo(width, height);
  ctx.lineTo(0, height);
  ctx.closePath();
  ctx.fill();
  ctx.restore();

  for (let x = 0; x < width; x += 8) {
    const ridge = Math.sin(x / 39 + frame / 110) * 3 + Math.sin(x / 23) * 2;
    fillPixelRect(
      ctx,
      x,
      baseY + ridge,
      8,
      height - baseY - ridge,
      "rgba(20,83,45,0.88)",
    );
    if ((x / 8) % 11 === 0) {
      fillPixelRect(
        ctx,
        x + 2,
        baseY + ridge + 11,
        7,
        2,
        "rgba(74,222,128,0.2)",
      );
    }
  }

  for (let x = 0; x < width; ) {
    const offset = (x * 17) % 43;
    const clumpX = x + offset;
    const clumpY = baseY + 12 + ((x * 11) % 22);
    const grassHeight = 4 + ((x + frame + offset) % 9);
    fillPixelRect(
      ctx,
      clumpX,
      clumpY - grassHeight,
      3,
      grassHeight,
      "rgba(187,247,208,0.28)",
    );
    fillPixelRect(
      ctx,
      clumpX + 4,
      clumpY - grassHeight + 2,
      2,
      Math.max(2, grassHeight - 1),
      "rgba(74,222,128,0.24)",
    );
    x += 52 + offset;
  }
}

function drawBuddyLandingPad(
  ctx: CanvasRenderingContext2D,
  frame: number,
  width: number,
  height: number,
): void {
  const x = width / 2;
  const y = height * 0.735 + Math.sin(frame / 30) * 1.2;

  ctx.save();
  ctx.fillStyle = "rgba(4, 20, 18, 0.32)";
  ctx.beginPath();
  ctx.ellipse(x, y + 18, 62, 13, 0, 0, Math.PI * 2);
  ctx.fill();

  ctx.fillStyle = "rgba(74, 222, 128, 0.16)";
  ctx.beginPath();
  ctx.ellipse(x, y + 14, 44, 8, 0, 0, Math.PI * 2);
  ctx.fill();

  ctx.strokeStyle = "rgba(187,247,208,0.22)";
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.ellipse(x, y + 13, 33, 6, 0, 0, Math.PI * 2);
  ctx.stroke();
  ctx.restore();
}

function drawBuddyHomeDoor(
  ctx: CanvasRenderingContext2D,
  frame: number,
  width: number,
  height: number,
  palette: Palette,
): void {
  const x = pctX(width, HOME_HOTSPOT.x);
  const y = pctY(height, HOME_HOTSPOT.y);
  const glow = 0.28 + Math.sin(frame / 32) * 0.08;

  ctx.save();
  ctx.fillStyle = `rgba(251,191,36,${glow})`;
  ctx.beginPath();
  ctx.ellipse(x, y + 14, 35, 15, 0, 0, Math.PI * 2);
  ctx.fill();

  const pathGlow = 0.36 + Math.sin(frame / 40) * 0.04;
  for (let i = 0; i < 6; i += 1) {
    const stepX = x + i * 9 + Math.sin(i * 1.7) * 4;
    const stepY = y + 32 + i * 5;
    ctx.fillStyle = `rgba(180,83,9,${pathGlow - i * 0.035})`;
    ctx.beginPath();
    ctx.ellipse(stepX, stepY, 8 - i * 0.45, 3.4, 0, 0, Math.PI * 2);
    ctx.fill();
  }

  fillPixelRect(ctx, x - 23, y - 1, 46, 30, "#92400E");
  fillPixelRect(ctx, x - 28, y + 25, 56, 5, "rgba(15,23,42,0.36)");
  fillPixelRect(ctx, x - 17, y - 15, 34, 8, palette.dark);
  fillPixelRect(ctx, x - 22, y - 7, 44, 8, palette.dark);
  fillPixelRect(ctx, x - 14, y - 29, 7, 12, "#475569");
  fillPixelRect(ctx, x - 7, y + 6, 14, 23, "#1E293B");
  fillPixelRect(ctx, x - 4, y + 10, 8, 19, "#0F172A");
  fillPixelRect(ctx, x + 7, y + 7, 8, 8, "#FDE68A");
  fillPixelRect(ctx, x + 9, y + 9, 4, 4, palette.light);
  fillPixelRect(ctx, x - 12, y + 31, 24, 3, "#FBBF24");

  fillPixelRect(ctx, x - 26, y - 43, 52, 12, "rgba(15,23,42,0.86)");
  fillPixelRect(ctx, x - 23, y - 40, 46, 2, palette.body);
  drawPixelText(ctx, "HOME", x, y - 36, palette.light);
  fillPixelRect(ctx, x - 2, y - 31, 4, 7, palette.body);

  const sparkleY = y - 7 + Math.sin(frame / 18) * 2;
  fillPixelRect(ctx, x + 27, sparkleY, 3, 3, "#FDE68A");
  fillPixelRect(ctx, x + 30, sparkleY + 3, 3, 3, palette.light);
  ctx.restore();
}

function drawHomePath(
  ctx: CanvasRenderingContext2D,
  frame: number,
  width: number,
  height: number,
): void {
  const startX = pctX(width, HOME_HOTSPOT.x) + 28;
  const startY = pctY(height, HOME_HOTSPOT.y) + 38;
  const endX = width / 2;
  const endY = height * 0.84;

  ctx.save();
  for (let i = 0; i < 12; i += 1) {
    const t = i / 11;
    const x = startX + (endX - startX) * t + Math.sin(i * 1.6) * 5;
    const y = startY + (endY - startY) * t + Math.sin(frame / 50 + i) * 1.1;
    ctx.fillStyle = `rgba(146,64,14,${0.32 - t * 0.1})`;
    ctx.beginPath();
    ctx.ellipse(x, y, 8 - t * 2, 3.2, 0, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.restore();
}

function drawObject(
  ctx: CanvasRenderingContext2D,
  item: BuddyWorldObject,
  frame: number,
  width: number,
  height: number,
): void {
  const x = pctX(width, item.x);
  const y = pctY(height, item.y);
  const tone = toneColor(item.tone);
  const pulse = Math.sin(frame / 24 + item.x) * 2;

  ctx.fillStyle = `${tone}1F`;
  ctx.beginPath();
  ctx.arc(x, y + 12, item.size + 9, 0, Math.PI * 2);
  ctx.fill();

  ctx.strokeStyle = `${tone}80`;
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.ellipse(x, y + item.size + 10, item.size + 9, 5, 0, 0, Math.PI * 2);
  ctx.stroke();

  switch (item.sprite) {
    case "task_grove":
      fillPixelRect(ctx, x - 5, y - 4, 10, 32, "#7C2D12");
      fillPixelRect(ctx, x - 17, y - 22 + pulse, 34, 18, "#22C55E");
      fillPixelRect(ctx, x - 10, y - 31 + pulse, 22, 14, "#86EFAC");
      fillPixelRect(ctx, x + 11, y - 11 + pulse, 9, 7, "#BBF7D0");
      fillPixelRect(ctx, x + 14, y - 8 + pulse, 6, 3, tone);
      break;
    case "memory_fireflies":
      for (let i = 0; i < 6; i += 1) {
        const fx = x + Math.sin(frame / 18 + i) * (8 + i * 2);
        const fy = y + Math.cos(frame / 15 + i) * 12;
        fillPixelRect(ctx, fx, fy, 4, 4, i % 2 === 0 ? "#FDE68A" : tone);
      }
      fillPixelRect(ctx, x - 14, y + 15, 28, 11, "#854D0E");
      fillPixelRect(ctx, x - 9, y + 10, 18, 6, "#F59E0B");
      break;
    case "observatory":
      fillPixelRect(ctx, x - 24, y + 13, 48, 18, "#334155");
      fillPixelRect(ctx, x - 18, y + 4, 36, 15, "#64748B");
      fillPixelRect(ctx, x - 10, y - 3, 20, 8, "#94A3B8");
      fillPixelRect(ctx, x - 4, y - 19, 8, 18, tone);
      fillPixelRect(ctx, x + 4, y - 14, 26, 6, "#CBD5E1");
      fillPixelRect(ctx, x + 27, y - 15, 5, 8, "#FDE68A");
      break;
    case "satellite":
      fillPixelRect(ctx, x - 8, y - 5 + pulse, 16, 10, "#CBD5E1");
      fillPixelRect(ctx, x - 26, y - 3 + pulse, 14, 6, tone);
      fillPixelRect(ctx, x + 12, y - 3 + pulse, 14, 6, tone);
      fillPixelRect(ctx, x - 1, y + 5 + pulse, 2, 18, "#94A3B8");
      break;
    case "git_vane":
      fillPixelRect(ctx, x - 2, y - 18, 4, 42, "#94A3B8");
      fillPixelRect(ctx, x - 14, y - 9, 28, 3, "#CBD5E1");
      fillPixelRect(ctx, x - 1, y - 22, 3, 30, "#CBD5E1");
      fillPixelRect(ctx, x - 18, y - 13, 8, 8, tone);
      fillPixelRect(ctx, x + 10, y - 13, 8, 8, "#86EFAC");
      fillPixelRect(ctx, x - 5, y - 26, 8, 8, "#F8FAFC");
      fillPixelRect(ctx, x - 4, y + 4, 8, 8, "#FDE68A");
      break;
    case "market_comet":
      fillPixelRect(ctx, x - 10, y - 7 + pulse, 20, 14, "#A855F7");
      fillPixelRect(ctx, x - 5, y - 3 + pulse, 10, 7, "#FDE68A");
      fillPixelRect(ctx, x - 29, y + pulse, 17, 3, "rgba(253,186,116,0.52)");
      fillPixelRect(ctx, x - 40, y + 3 + pulse, 9, 2, "rgba(253,186,116,0.32)");
      break;
    case "seed":
      fillPixelRect(ctx, x - 3, y, 6, 20, "#15803D");
      fillPixelRect(ctx, x - 15, y - 12, 14, 10, "#22C55E");
      fillPixelRect(ctx, x + 1, y - 16, 15, 10, "#86EFAC");
      break;
  }

  const glint = 0.38 + Math.sin(frame / 20 + item.x) * 0.18;
  fillPixelRect(
    ctx,
    x + item.size + 4,
    y - item.size + pulse,
    3,
    3,
    `rgba(253,224,71,${glint})`,
  );
  fillPixelRect(
    ctx,
    x + item.size + 7,
    y - item.size + 3 + pulse,
    3,
    3,
    `rgba(255,255,255,${glint})`,
  );
}

function drawVitality(
  ctx: CanvasRenderingContext2D,
  world: BuddyWorldState,
  frame: number,
  width: number,
  height: number,
): void {
  const groundY = height * 0.8;

  if (world.vitality === "lush") {
    for (let i = 8; i < width; ) {
      const offset = (i * 11) % 37;
      const bloomY = groundY + 16 + Math.sin(frame / 30 + i) * 2;
      fillPixelRect(ctx, i + 10, bloomY, 4, 4, "#FDE68A");
      fillPixelRect(ctx, i + 14, bloomY - 4, 4, 4, "#F9A8D4");
      fillPixelRect(ctx, i + 14, bloomY + 4, 4, 4, "#86EFAC");
      i += 56 + offset;
    }
    return;
  }

  if (world.vitality === "growing") {
    for (let i = 20; i < width; ) {
      const offset = (i * 13) % 41;
      const sway = Math.sin(frame / 32 + i) * 2;
      fillPixelRect(ctx, i + sway, groundY + 12, 4, 18, "#16A34A");
      fillPixelRect(ctx, i - 9 + sway, groundY + 14, 12, 5, "#86EFAC");
      fillPixelRect(ctx, i + 3 + sway, groundY + 8, 14, 5, "#4ADE80");
      i += 66 + offset;
    }
    return;
  }

  for (let i = 18; i < width; ) {
    const offset = (i * 13) % 47;
    const x = i + offset;
    const sway = Math.sin(frame / 36 + x) * 3;
    const heightOffset = (i * 7) % 10;
    fillPixelRect(ctx, x + sway, groundY + 10 - heightOffset, 4, 18, "#365314");
    fillPixelRect(ctx, x - 6 + sway, groundY + 16, 12, 3, "#854D0E");
    if (i % 5 === 0) {
      fillPixelRect(
        ctx,
        x + 8 + sway,
        groundY + 22,
        3,
        3,
        "rgba(239,68,68,0.58)",
      );
    }
    i += 104 + offset;
  }
}

function clampBuddySceneX(x: number): number {
  return Math.max(BUDDY_MIN_X, Math.min(BUDDY_MAX_X, x));
}

function buildBuddyShowcaseTargets(
  world: BuddyWorldState,
): BuddyShowcaseTargetCandidate[] {
  return world.objects.map((item) => ({
    id: item.id,
    x: item.x,
    y: item.y,
    label: item.label,
    sprite: item.sprite,
  }));
}

function buildBuddyWaypoints(world: BuddyWorldState): BuddyWaypoint[] {
  return [
    {
      id: "center",
      x: BUDDY_CENTER_X,
      y: 76,
      label: "clearing",
      reaction: "Buddy wanders back to the clearing.",
    },
    {
      id: "home",
      x: HOME_HOTSPOT.x,
      y: HOME_HOTSPOT.y,
      label: "home",
      reaction: "Buddy checks the front door lights.",
    },
    {
      id: "celestial",
      x: world.celestialX,
      y: world.celestialY,
      label: world.celestialLabel,
      reaction: `Buddy tracks the ${world.celestialLabel.toLowerCase()}.`,
    },
    ...world.objects.map((item) => ({
      id: item.id,
      x: item.x,
      y: item.y,
      label: item.label,
      reaction: `Buddy inspects ${item.label.toLowerCase()}.`,
    })),
    {
      id: "weather",
      x: world.weatherX,
      y: world.weatherY,
      label: world.weatherLabel,
      reaction: `Buddy watches ${world.weatherLabel.toLowerCase()}.`,
    },
  ];
}

function pickNextWaypointIndex(
  waypoints: BuddyWaypoint[],
  currentIndex: number,
): number {
  if (waypoints.length <= 1) return 0;

  const roll = Math.random();
  if (roll < 0.24) return 0;

  let nextIndex = currentIndex;
  while (nextIndex === currentIndex) {
    nextIndex = Math.floor(Math.random() * waypoints.length);
  }
  return nextIndex;
}

function randomIdleReaction(): string {
  return RANDOM_IDLE_REACTIONS[
    Math.floor(Math.random() * RANDOM_IDLE_REACTIONS.length)
  ];
}

function prefersReducedMotion(): boolean {
  if (typeof window === "undefined") return false;
  if (typeof window.matchMedia !== "function") return false;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function resolveBuddyWorldSpeechOverride(args: {
  activeSpeechText: string | null;
  showcaseActive: boolean;
  showcaseSpeech: string | null;
  reaction: string | null;
}): string | null {
  if (args.activeSpeechText !== null) return args.activeSpeechText;
  if (args.showcaseActive) return args.showcaseSpeech;
  return args.reaction;
}

function drawScene(
  ctx: CanvasRenderingContext2D,
  world: BuddyWorldState,
  palette: Palette,
  frame: number,
  width: number,
  height: number,
): void {
  ctx.clearRect(0, 0, width, height);
  ctx.imageSmoothingEnabled = false;

  const gradient = ctx.createLinearGradient(0, 0, 0, height);
  if (world.phase === "night") {
    gradient.addColorStop(0, "#111827");
    gradient.addColorStop(0.55, "#312E81");
    gradient.addColorStop(1, "#064E3B");
  } else if (world.phase === "evening") {
    gradient.addColorStop(0, "#7C2D12");
    gradient.addColorStop(0.54, "#6D28D9");
    gradient.addColorStop(1, "#14532D");
  } else if (world.phase === "morning") {
    gradient.addColorStop(0, "#0EA5E9");
    gradient.addColorStop(0.52, "#F59E0B");
    gradient.addColorStop(1, "#166534");
  } else {
    gradient.addColorStop(0, "#38BDF8");
    gradient.addColorStop(0.58, "#93C5FD");
    gradient.addColorStop(1, "#15803D");
  }
  ctx.fillStyle = gradient;
  ctx.fillRect(0, 0, width, height);

  drawDistantHills(ctx, world, frame, width, height);

  for (let i = 0; i < 52; i += 1) {
    const sx = (i * 47 + frame * 0.08) % width;
    const sy = (i * 31) % (height * 0.58);
    const alpha = world.phase === "night" ? 0.72 : 0.22;
    fillPixelRect(
      ctx,
      sx,
      sy,
      i % 5 === 0 ? 3 : 2,
      i % 7 === 0 ? 3 : 2,
      `rgba(255,255,255,${alpha})`,
    );
  }

  drawCelestial(ctx, world, frame, width, height);
  drawWeather(ctx, world, frame, width, height);

  drawGround(ctx, frame, width, height);
  drawHomePath(ctx, frame, width, height);

  drawBuddyHomeDoor(ctx, frame, width, height, palette);
  drawVitality(ctx, world, frame, width, height);
  drawBuddyLandingPad(ctx, frame, width, height);

  for (const item of world.objects) {
    drawObject(ctx, item, frame, width, height);
  }
}

export const BuddyWorld: React.FC<BuddyWorldProps> = ({
  palette,
  stage,
  state,
  pulse,
  pet,
  nowPlaying,
  activeQuest,
  activeSpeech,
  setupNeeded,
  compact = false,
  homeDoorDisabled = false,
  onCanvasEvent,
  onCare,
  onOpenPage,
  onRunMode,
  onDismissSetup,
  onSpeechControl,
  now,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [currentTime, setCurrentTime] = useState(() => now ?? new Date());
  const [reaction, setReaction] = useState<string | null>(null);
  const [activeWaypointIndex, setActiveWaypointIndex] = useState(0);
  const [lastWaypoint, setLastWaypoint] = useState<BuddyWaypoint | null>(null);
  const [randomPose, setRandomPose] = useState<BuddyRandomPose>("idle");
  const [showcaseRun, setShowcaseRun] = useState<BuddyShowcaseRun | null>(null);
  const [lastShowcaseKind, setLastShowcaseKind] =
    useState<BuddyShowcaseKind | null>(null);
  const [runtimeShowcaseEventIds, setRuntimeShowcaseEventIds] = useState<
    string[]
  >([]);
  const [idleTick, setIdleTick] = useState(0);
  const [idleGraceUntilMs] = useState(
    () => Date.now() + BUDDY_SHOWCASE_INITIAL_GRACE_MS,
  );
  const [nextIdleShowcaseAtMs, setNextIdleShowcaseAtMs] = useState(0);
  const [nextRuntimeShowcaseAtMs, setNextRuntimeShowcaseAtMs] = useState(0);
  const [reducedMotion, setReducedMotion] = useState(prefersReducedMotion);

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (typeof window.matchMedia !== "function") {
      setReducedMotion(false);
      return;
    }

    const media = window.matchMedia("(prefers-reduced-motion: reduce)") as {
      matches: boolean;
      addEventListener?: (type: "change", listener: () => void) => void;
      removeEventListener?: (type: "change", listener: () => void) => void;
      addListener?: (listener: () => void) => void;
      removeListener?: (listener: () => void) => void;
    };
    const updateReducedMotion = () => setReducedMotion(media.matches);
    updateReducedMotion();
    if (typeof media.addEventListener === "function") {
      media.addEventListener("change", updateReducedMotion);
      return () => {
        if (typeof media.removeEventListener === "function") {
          media.removeEventListener("change", updateReducedMotion);
        }
      };
    }
    if (typeof media.addListener === "function") {
      media.addListener(updateReducedMotion);
      return () => {
        if (typeof media.removeListener === "function") {
          media.removeListener(updateReducedMotion);
        }
      };
    }
  }, []);

  useEffect(() => {
    if (now) {
      setCurrentTime(now);
      return;
    }
    const timer = window.setInterval(() => setCurrentTime(new Date()), 60_000);
    return () => window.clearInterval(timer);
  }, [now]);

  useEffect(() => {
    if (!reaction) return;
    const timer = window.setTimeout(() => setReaction(null), 5000);
    return () => window.clearTimeout(timer);
  }, [reaction]);

  useEffect(() => {
    if (randomPose === "idle") return;
    const timer = window.setTimeout(() => setRandomPose("idle"), 2600);
    return () => window.clearTimeout(timer);
  }, [randomPose]);

  const world = useMemo(
    () =>
      buildBuddyWorldState({
        now: currentTime,
        pulse,
        pet,
        nowPlaying,
        activeQuest,
      }),
    [activeQuest, currentTime, nowPlaying, pet, pulse],
  );
  const waypoints = useMemo(() => buildBuddyWaypoints(world), [world]);
  const showcaseTargets = useMemo(
    () => buildBuddyShowcaseTargets(world),
    [world],
  );
  const activeWaypoint = waypoints[activeWaypointIndex % waypoints.length];
  const characterSceneX = clampBuddySceneX(
    showcaseRun ? showcaseRun.target.x : activeWaypoint.x,
  );

  useEffect(() => {
    setActiveWaypointIndex(0);
    setLastWaypoint(null);
  }, [world.headline]);

  const startShowcase = useCallback(
    (strongRuntimeTrigger: boolean) => {
      if (showcaseRun) return false;
      const nowMs = Date.now();
      const run = createBuddyShowcaseRun({
        targets: showcaseTargets,
        nowPlaying,
        activeSpeechVisible: Boolean(activeSpeech) || Boolean(reaction),
        pet,
        nowMs,
        idleCooldownUntilMs: nextIdleShowcaseAtMs,
        runtimeCooldownUntilMs: nextRuntimeShowcaseAtMs,
        idleGraceUntilMs,
        lastShowcaseKind,
        runtimeShowcaseEventIds,
        strongRuntimeTrigger,
        world: {
          phase: world.phase,
          weather: world.weather,
        },
        pulse,
      });
      if (!run) return false;
      setShowcaseRun(run);
      setLastWaypoint(null);
      setLastShowcaseKind(run.kind);
      if (strongRuntimeTrigger && nowPlaying?.id) {
        setRuntimeShowcaseEventIds((eventIds) =>
          [
            nowPlaying.id,
            ...eventIds.filter((eventId) => eventId !== nowPlaying.id),
          ].slice(0, MAX_RUNTIME_SHOWCASE_EVENT_IDS),
        );
      }
      if (strongRuntimeTrigger) {
        setNextRuntimeShowcaseAtMs(nowMs + BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS);
      } else {
        setNextIdleShowcaseAtMs(nowMs + BUDDY_SHOWCASE_IDLE_COOLDOWN_MS);
      }
      return true;
    },
    [
      activeSpeech,
      idleGraceUntilMs,
      lastShowcaseKind,
      runtimeShowcaseEventIds,
      nextIdleShowcaseAtMs,
      nextRuntimeShowcaseAtMs,
      nowPlaying,
      pet,
      pulse,
      reaction,
      showcaseRun,
      showcaseTargets,
      world.phase,
      world.weather,
    ],
  );

  useEffect(() => {
    if (activeSpeech ?? reaction ?? showcaseRun) return;
    const delay = 4200 + Math.random() * 7200;
    const timer = window.setTimeout(() => {
      const roll = Math.random();
      if (roll < 0.18 && startShowcase(false)) return;

      if (roll < 0.34) {
        setRandomPose(
          RANDOM_POSES[Math.floor(Math.random() * RANDOM_POSES.length)],
        );
        setReaction(randomIdleReaction());
      } else if (roll < 0.46) {
        setLastWaypoint(null);
      } else {
        setLastWaypoint(null);
        setActiveWaypointIndex((index) =>
          pickNextWaypointIndex(waypoints, index),
        );
      }
      setIdleTick((tick) => tick + 1);
    }, delay);
    return () => window.clearTimeout(timer);
  }, [activeSpeech, idleTick, reaction, showcaseRun, startShowcase, waypoints]);

  useEffect(() => {
    if (activeSpeech ?? reaction ?? showcaseRun) return;
    if (lastWaypoint?.id === activeWaypoint.id) return;
    const timer = window.setTimeout(() => {
      setLastWaypoint(activeWaypoint);
      if (Math.random() < 0.72) {
        setReaction(activeWaypoint.reaction);
      }
    }, 2200);
    return () => window.clearTimeout(timer);
  }, [activeSpeech, activeWaypoint, lastWaypoint, reaction, showcaseRun]);

  useEffect(() => {
    if (
      activeSpeech !== null ||
      reaction !== null ||
      showcaseRun !== null ||
      nowPlaying === null ||
      !hasBuddyShowcaseRuntimeTrigger(nowPlaying)
    ) {
      return;
    }
    if (nowPlaying.id && runtimeShowcaseEventIds.includes(nowPlaying.id)) {
      return;
    }

    const nowMs = Date.now();
    if (nowMs < nextRuntimeShowcaseAtMs) {
      const timer = window.setTimeout(
        () => startShowcase(true),
        nextRuntimeShowcaseAtMs - nowMs,
      );
      return () => window.clearTimeout(timer);
    }

    startShowcase(true);
  }, [
    activeSpeech,
    runtimeShowcaseEventIds,
    nextRuntimeShowcaseAtMs,
    nowPlaying,
    reaction,
    showcaseRun,
    startShowcase,
  ]);

  useEffect(() => {
    if (!showcaseRun) return;
    const nowMs = Date.now();
    const elapsedMs = nowMs - showcaseRun.phaseStartedAtMs;
    const remainingMs = Math.max(
      0,
      BUDDY_SHOWCASE_PHASE_DURATIONS_MS[showcaseRun.phase] - elapsedMs,
    );
    const timer = window.setTimeout(() => {
      const currentNowMs = Date.now();
      const advanced = advanceBuddyShowcasePhase({
        run: showcaseRun,
        nowMs: currentNowMs,
      });
      setShowcaseRun(advanced);
      if (!advanced) {
        setNextIdleShowcaseAtMs(currentNowMs + BUDDY_SHOWCASE_IDLE_COOLDOWN_MS);
      }
    }, remainingMs + 16);
    return () => window.clearTimeout(timer);
  }, [showcaseRun]);

  useEffect(() => {
    let frame = 0;
    let raf = 0;
    const render = () => {
      if (document.hidden) {
        raf = window.requestAnimationFrame(render);
        return;
      }

      frame += 1;
      const canvas = canvasRef.current;
      const ctx = canvas?.getContext("2d");
      if (canvas && ctx) {
        const rect = canvas.getBoundingClientRect();
        const cssWidth = Math.max(1, Math.round(rect.width || 720));
        const cssHeight = Math.max(
          1,
          Math.round(rect.height || (compact ? 190 : 260)),
        );
        const ratio = window.devicePixelRatio || 1;
        const targetWidth = Math.round(cssWidth * ratio);
        const targetHeight = Math.round(cssHeight * ratio);
        if (canvas.width !== targetWidth || canvas.height !== targetHeight) {
          canvas.width = targetWidth;
          canvas.height = targetHeight;
        }
        ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
        drawScene(ctx, world, palette, frame, cssWidth, cssHeight);
        if (showcaseRun) {
          drawShowcaseEvent({
            ctx,
            run: showcaseRun,
            world,
            palette,
            frame,
            width: cssWidth,
            height: cssHeight,
            compact,
            reducedMotion,
          });
        }
      }
      raf = window.requestAnimationFrame(render);
    };
    raf = window.requestAnimationFrame(render);
    return () => window.cancelAnimationFrame(raf);
  }, [compact, palette, reducedMotion, showcaseRun, world]);

  const handleCelestialClick = () => {
    if (!showcaseRun) {
      setActiveWaypointIndex(
        Math.max(
          0,
          waypoints.findIndex((point) => point.id === "celestial"),
        ),
      );
    }
    if (world.phase === "night") {
      onCare("sleep");
      if (!showcaseRun) {
        setReaction("Buddy curls up under the moon and saves energy.");
      }
      return;
    }
    onCare("play", "scroll");
    if (!showcaseRun) {
      setReaction("Buddy catches a warm sunbeam and opens the focus scroll.");
    }
  };

  const handleWeatherClick = () => {
    if (!showcaseRun) {
      setActiveWaypointIndex(
        Math.max(
          0,
          waypoints.findIndex((point) => point.id === "weather"),
        ),
      );
    }
    if (world.weather === "storm") {
      onOpenPage({ type: "stats" });
      if (!showcaseRun) {
        setReaction("Buddy marked the storm front for investigation.");
      }
      return;
    }
    if (world.weather === "rain") {
      onOpenPage({ type: "knowledge_graph" });
      if (!showcaseRun) {
        setReaction("Buddy follows the rain into the memory garden.");
      }
      return;
    }
    onCare("pet");
    if (!showcaseRun) {
      setReaction("Buddy chirps back at the sky.");
    }
  };

  const handleHomeClick = () => {
    if (!showcaseRun) {
      setActiveWaypointIndex(
        Math.max(
          0,
          waypoints.findIndex((point) => point.id === "home"),
        ),
      );
    }
    if (homeDoorDisabled) {
      if (!showcaseRun) {
        setReaction("Buddy is already home.");
      }
      return;
    }
    onOpenPage({ type: "buddy" });
    if (!showcaseRun) {
      setReaction("Buddy opens the front door.");
    }
  };

  const showcasePose =
    showcaseRun !== null && showcaseRun.phase !== "travel"
      ? showcaseRun.pose
      : null;
  const characterPose: BuddyScenePose = showcasePose ?? randomPose;
  const speechOverride = resolveBuddyWorldSpeechOverride({
    activeSpeechText: activeSpeech?.text ?? null,
    showcaseActive: showcaseRun !== null,
    showcaseSpeech: showcaseRun?.speech ?? null,
    reaction,
  });
  const speechSource = activeSpeech
    ? "active"
    : showcaseRun
      ? "showcase"
      : reaction
        ? "reaction"
        : "none";

  return (
    <section
      className={classNames(styles.scene, { [styles.compact]: compact })}
      data-phase={world.phase}
      data-weather={world.weather}
      data-vitality={world.vitality}
      data-showcase={showcaseRun?.kind ?? "none"}
      data-showcase-phase={showcaseRun?.phase ?? "idle"}
      data-speech-priority="backend-showcase-local"
      data-speech-source={speechSource}
      data-testid="buddy-world"
      aria-label={`Buddy virtual scene: ${world.phaseLabel}. ${world.vitalityLabel}.`}
    >
      <canvas
        ref={canvasRef}
        className={styles.canvas}
        data-testid="buddy-world-canvas"
      />

      <button
        type="button"
        className={classNames(styles.hotspot, styles.celestialHotspot)}
        style={{ left: `${world.celestialX}%`, top: `${world.celestialY}%` }}
        onClick={handleCelestialClick}
        aria-label={`${world.celestialAction} with ${world.celestialLabel}`}
        title={`${world.celestialAction} with ${world.celestialLabel}`}
      />

      <button
        type="button"
        className={classNames(styles.hotspot, styles.weatherHotspot)}
        style={{ left: `${world.weatherX}%`, top: `${world.weatherY}%` }}
        onClick={handleWeatherClick}
        aria-label={`Interact with ${world.weatherLabel}`}
        title={world.weatherLabel}
      />

      <button
        type="button"
        className={classNames(styles.hotspot, styles.homeHotspot)}
        style={{ left: `${HOME_HOTSPOT.x}%`, top: `${HOME_HOTSPOT.y}%` }}
        onClick={handleHomeClick}
        aria-label={
          homeDoorDisabled ? "Buddy home entrance" : "Open Buddy home"
        }
        title={homeDoorDisabled ? "Buddy is home" : "Open Buddy home"}
      />

      {world.objects.map((item) => (
        <button
          key={item.id}
          type="button"
          className={classNames(styles.objectHotspot, TONE_CLASS[item.tone])}
          style={{ left: `${item.x}%`, top: `${item.y}%` }}
          onClick={() => {
            if (!showcaseRun) {
              setActiveWaypointIndex(
                Math.max(
                  0,
                  waypoints.findIndex((point) => point.id === item.id),
                ),
              );
            }
            onOpenPage(item.page);
            if (!showcaseRun) {
              setReaction(`Buddy hops toward ${item.label.toLowerCase()}.`);
            }
          }}
          aria-label={`Open ${item.label}`}
          title={`${item.label}: ${item.description}`}
        >
          <span className={styles.objectTooltip}>
            <span className={styles.objectLabel}>{item.label}</span>
            <span className={styles.objectValue}>{item.value}</span>
          </span>
        </button>
      ))}

      {lastWaypoint && (
        <div
          className={styles.waypointPing}
          style={{ left: `${lastWaypoint.x}%`, top: `${lastWaypoint.y}%` }}
          aria-hidden
        />
      )}

      <BuddyCharacter
        state={state}
        stage={stage}
        palette={palette}
        displaySize={compact ? 230 : 282}
        sceneXPercent={characterSceneX}
        scenePose={characterPose}
        speechText={speechOverride}
        speechControls={activeSpeech ? activeSpeech.controls : undefined}
        randomizeBubblePosition
        onCanvasEvent={onCanvasEvent}
        onSpeechControl={activeSpeech ? onSpeechControl : undefined}
      />

      {setupNeeded && (
        <div className={styles.setupDock}>
          {SETUP_MODE_ACTIONS.map((item) => (
            <button
              key={item.mode}
              type="button"
              className={styles.sceneButton}
              onClick={() => onRunMode(item.mode)}
            >
              {item.label}
            </button>
          ))}
          <button
            type="button"
            className={styles.sceneButtonGhost}
            onClick={onDismissSetup}
          >
            Later
          </button>
        </div>
      )}

      <div className={styles.careDock} aria-label="Buddy scene care actions">
        <button
          type="button"
          className={styles.sceneButton}
          aria-label="Water Buddy garden"
          onClick={() => onCare("feed")}
        >
          🍜
        </button>
        <button
          type="button"
          className={styles.sceneButton}
          aria-label="Hunt bugs with Buddy"
          onClick={() => onCare("play", "bug")}
        >
          🐛
        </button>
        <button
          type="button"
          className={styles.sceneButton}
          aria-label="Clean Buddy"
          onClick={() => onCare("clean")}
        >
          🧼
        </button>
        <button
          type="button"
          className={styles.sceneButton}
          aria-label="Let Buddy rest"
          onClick={() => onCare("sleep")}
        >
          😴
        </button>
      </div>
    </section>
  );
};
