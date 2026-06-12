import { useCallback, useEffect, useRef, useState } from "react";
import type { BuddyWorldState } from "../buddyWorldModel";
import type {
  BuddySpeechMemory,
  BuddyWorldSpeechCandidate,
} from "../buddySpeech";
import { pickBuddySpeechLine } from "../buddySpeech";
import {
  BUDDY_WORLD_ARC_DEFS,
  advanceBuddyArcRun,
  buddyArcLanternLitCount,
  chooseBuddyWorldArc,
  createBuddyArcRun,
  currentBuddyArcStep,
  seasonFirstMemoKey,
  type BuddyArcRun,
  type BuddyArcStepDef,
} from "../buddyWorldArcs";
import {
  buddyWorldDayKey,
  readBuddyWorldMemos,
  writeBuddyWorldMemos,
} from "../buddyWorldMemos";

export interface UseBuddyWorldArcsArgs {
  world: BuddyWorldState;
  name: string;
  busy: boolean;
  showcaseActive: boolean;
  showcaseIsRuntime: boolean;
  reducedMotion: boolean;
  speechMemory: BuddySpeechMemory;
  onArcStarted?: () => void;
}

export interface UseBuddyWorldArcsResult {
  arcRun: BuddyArcRun | null;
  arcStep: BuddyArcStepDef | null;
  arcLine: BuddyWorldSpeechCandidate | null;
  arcLanternLitCount: number | null;
  cancelArc: () => void;
}

export function useBuddyWorldArcs(
  args: UseBuddyWorldArcsArgs,
): UseBuddyWorldArcsResult {
  const {
    world,
    name,
    busy,
    showcaseActive,
    showcaseIsRuntime,
    reducedMotion,
    speechMemory,
    onArcStarted,
  } = args;
  const [arcRun, setArcRun] = useState<BuddyArcRun | null>(null);
  const [arcLine, setArcLine] = useState<BuddyWorldSpeechCandidate | null>(
    null,
  );
  const previousPhaseRef = useRef<BuddyWorldState["phase"] | null>(null);
  const previousWeatherRef = useRef<BuddyWorldState["weather"] | null>(null);

  const cancelArc = useCallback(() => {
    setArcRun(null);
    setArcLine(null);
  }, []);

  useEffect(() => {
    if (busy && arcRun) cancelArc();
  }, [busy, arcRun, cancelArc]);

  useEffect(() => {
    if (showcaseActive && showcaseIsRuntime && arcRun) cancelArc();
  }, [showcaseActive, showcaseIsRuntime, arcRun, cancelArc]);

  useEffect(() => {
    const previousPhase = previousPhaseRef.current;
    const previousWeather = previousWeatherRef.current;
    previousPhaseRef.current = world.phase;
    previousWeatherRef.current = world.weather;

    if (arcRun) return;
    if (busy) return;
    if (showcaseActive && showcaseIsRuntime) return;

    const now = new Date();
    const kind = chooseBuddyWorldArc({
      previousPhase,
      phase: world.phase,
      previousWeather,
      weather: world.weather,
      layers: world.atmosphere.layers,
      memos: readBuddyWorldMemos(),
      dayKey: buddyWorldDayKey(now),
      year: now.getFullYear(),
      busy,
    });
    if (!kind) return;

    if (BUDDY_WORLD_ARC_DEFS[kind].oncePerDay) {
      writeBuddyWorldMemos({
        lastArcDates: { [kind]: buddyWorldDayKey(now) },
      });
    } else {
      writeBuddyWorldMemos({
        seasonFirstsSeen: [seasonFirstMemoKey(kind, now.getFullYear())],
      });
    }
    setArcRun(createBuddyArcRun(kind, Date.now()));
    setArcLine(null);
    onArcStarted?.();
  }, [world, arcRun, busy, showcaseActive, showcaseIsRuntime, onArcStarted]);

  useEffect(() => {
    if (!arcRun) return;
    const step = currentBuddyArcStep(arcRun);
    if (!step) {
      cancelArc();
      return;
    }
    const stepDurationMs = reducedMotion
      ? Math.round(step.durationMs * 1.2)
      : step.durationMs;
    const elapsedMs = Date.now() - arcRun.stepStartedAtMs;
    const timer = window.setTimeout(
      () => {
        setArcRun((current) => {
          if (
            !current ||
            current.id !== arcRun.id ||
            current.stepIndex !== arcRun.stepIndex ||
            current.finale !== arcRun.finale
          ) {
            return current;
          }
          const settledNowMs = Math.max(
            Date.now(),
            current.stepStartedAtMs + step.durationMs,
          );
          return advanceBuddyArcRun(current, settledNowMs, world.weather);
        });
      },
      Math.max(16, stepDurationMs - elapsedMs),
    );
    return () => window.clearTimeout(timer);
  }, [arcRun, cancelArc, reducedMotion, world.weather]);

  useEffect(() => {
    setArcLine(null);
    if (!arcRun) return;
    const step = currentBuddyArcStep(arcRun);
    if (!step || step.beats.length === 0) return;
    const timers = step.beats.map((beat) =>
      window.setTimeout(
        () => {
          setArcLine({
            text: pickBuddySpeechLine(
              speechMemory,
              beat.poolKey,
              beat.lines,
              name,
            ),
            style: beat.style,
          });
        },
        Math.max(0, arcRun.stepStartedAtMs + beat.atMs - Date.now()),
      ),
    );
    return () => {
      for (const timer of timers) window.clearTimeout(timer);
    };
  }, [arcRun, name, speechMemory]);

  const arcStep = arcRun ? currentBuddyArcStep(arcRun) : null;

  return {
    arcRun,
    arcStep,
    arcLine: arcRun ? arcLine : null,
    arcLanternLitCount: buddyArcLanternLitCount(arcRun),
    cancelArc,
  };
}
