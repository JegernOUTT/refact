import { useEffect, useMemo, useRef, useState } from "react";
import type { BuddyScenePose } from "../types";
import type { BuddyWorldState } from "../buddyWorldModel";
import {
  KURO_FLEE_LINES,
  SHIRO_INTRO_LINES,
  SHIRO_TICK_MS,
  createShiroState,
  kuroCompanion,
  kuroDayActive,
  shiroCompanion,
  sootCompanions,
  stepKuroState,
  stepShiroState,
  type BuddyWorldCompanion,
  type KuroState,
  type ShiroState,
} from "../buddyCompanions";
import {
  buddyWorldDayKey,
  readBuddyWorldMemos,
  writeBuddyWorldMemos,
} from "../buddyWorldMemos";

export interface UseBuddyCompanionsArgs {
  world: BuddyWorldState;
  stageNumber: number;
  name: string;
  buddyX: number;
  buddyY: number;
  buddyPose: BuddyScenePose;
  longActionActive: boolean;
  sleeping: boolean;
  gatherActive: boolean;
  reducedMotion: boolean;
  onShiroIntro?: (line: string) => void;
  onKuroFlee?: (line: string) => void;
}

export interface UseBuddyCompanionsResult {
  companions: BuddyWorldCompanion[];
}

export function useBuddyCompanions(
  args: UseBuddyCompanionsArgs,
): UseBuddyCompanionsResult {
  const {
    world,
    stageNumber,
    name,
    buddyX,
    buddyY,
    buddyPose,
    longActionActive,
    sleeping,
    gatherActive,
    reducedMotion,
    onShiroIntro,
    onKuroFlee,
  } = args;
  const shiroEnabled = stageNumber >= 2;
  const [shiroState, setShiroState] = useState<ShiroState | null>(null);
  const [kuroState, setKuroState] = useState<KuroState>({
    mode: "away",
    sinceMs: 0,
  });
  const introFiredRef = useRef(false);
  const contextRef = useRef({
    buddyX,
    buddyY,
    buddyPose,
    longActionActive,
    sleeping,
    storm: world.weather === "storm",
    gatherActive,
  });

  useEffect(() => {
    contextRef.current = {
      buddyX,
      buddyY,
      buddyPose,
      longActionActive,
      sleeping,
      storm: world.weather === "storm",
      gatherActive,
    };
  }, [
    buddyX,
    buddyY,
    buddyPose,
    longActionActive,
    sleeping,
    world.weather,
    gatherActive,
  ]);

  useEffect(() => {
    if (!shiroEnabled || introFiredRef.current) return;
    introFiredRef.current = true;
    setShiroState(
      (current) => current ?? createShiroState(Date.now(), buddyX, buddyY),
    );
    const memos = readBuddyWorldMemos();
    if (!memos.shiroIntroSeen) {
      writeBuddyWorldMemos({ shiroIntroSeen: true });
      const line =
        SHIRO_INTRO_LINES[
          Math.floor(Math.random() * SHIRO_INTRO_LINES.length)
        ] ?? SHIRO_INTRO_LINES[0];
      onShiroIntro?.(line(name));
    }
  }, [shiroEnabled, buddyX, buddyY, name, onShiroIntro]);

  useEffect(() => {
    if (!shiroEnabled) return;
    const tickMs = reducedMotion ? SHIRO_TICK_MS * 2 : SHIRO_TICK_MS;
    const timer = window.setInterval(() => {
      const context = contextRef.current;
      const nowMs = Date.now();
      setShiroState((current) =>
        current
          ? stepShiroState(current, {
              buddyX: context.buddyX,
              buddyY: context.buddyY,
              buddyPose: context.buddyPose,
              longActionActive: context.longActionActive,
              sleeping: context.sleeping,
              storm: context.storm,
              nowMs,
            })
          : createShiroState(nowMs, context.buddyX, context.buddyY),
      );
      setKuroState((current) => {
        const dayKey = buddyWorldDayKey(new Date());
        const stepped = stepKuroState(current, {
          active: kuroDayActive(dayKey, world.season),
          gatherActive: context.gatherActive,
          buddyX: context.buddyX,
          nowMs,
        });
        if (stepped.fledNow) {
          const line =
            KURO_FLEE_LINES[
              Math.floor(Math.random() * KURO_FLEE_LINES.length)
            ] ?? KURO_FLEE_LINES[0];
          onKuroFlee?.(line(name));
        }
        return stepped.state;
      });
    }, tickMs);
    return () => window.clearInterval(timer);
  }, [shiroEnabled, reducedMotion, world.season, name, onKuroFlee]);

  const companions = useMemo(() => {
    const list: BuddyWorldCompanion[] = [];
    if (shiroEnabled && shiroState) list.push(shiroCompanion(shiroState));
    list.push(
      ...sootCompanions({
        phase: world.phase,
        weather: world.weather,
        layers: world.atmosphere.layers,
        buddyX,
        dayKey: buddyWorldDayKey(new Date()),
      }),
    );
    const kuro = kuroCompanion(kuroState);
    if (kuro) list.push(kuro);
    return list;
  }, [
    shiroEnabled,
    shiroState,
    world.phase,
    world.weather,
    world.atmosphere.layers,
    buddyX,
    kuroState,
  ]);

  return { companions };
}
