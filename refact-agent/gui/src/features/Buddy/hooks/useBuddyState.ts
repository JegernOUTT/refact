import { useCallback, useEffect, useReducer, useRef } from "react";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  createInitialSemanticState,
  reduceSemanticState,
  type SemanticAction,
} from "../state";
import {
  selectBuddySnapshot,
  selectNowPlaying,
  clearNowPlaying,
} from "../buddySlice";
import { SIGNALS, STAGES, SKILLS } from "../constants";
import { anxietyFromNeglect } from "../buddyUtils";
import { useUpdateBuddySettingsMutation } from "../../../services/refact/buddy";
import type { BuddySemanticState, BuddyEvent, MoodType } from "../types";

const SEMANTIC_MOOD_MAP: Record<string, MoodType> = {
  sleepy: "sleepy",
  restless: "curious",
  questing: "focused",
  hungry: "eating",
  grimy: "concerned",
  needy: "concerned",
  playful: "happy",
  excited: "celebrate",
  neutral: "idle",
  calm: "idle",
  cheerful: "happy",
  worried: "concerned",
  happy: "happy",
  curious: "curious",
  focused: "focused",
};

function semanticMood(mood: string, currentMood: MoodType): MoodType {
  const normalizedMood = mood.toLowerCase();
  if (Object.prototype.hasOwnProperty.call(SEMANTIC_MOOD_MAP, normalizedMood)) {
    return SEMANTIC_MOOD_MAP[normalizedMood] as MoodType;
  }
  return currentMood;
}

export function clinginessFromAffection(affection: number): number {
  return Math.min(100, Math.max(0, 100 - affection));
}

export interface BuddyStateHandle {
  state: BuddySemanticState;
  signal: (signalType: string) => void;
  addXP: (amount: number) => void;
  pet: () => void;
  rename: (name: string) => void;
  nextPalette: () => void;
  reset: () => void;
  handleCanvasEvent: (event: BuddyEvent) => void;
  onBuddyEvent?: (event: BuddyEvent) => void;
}

export function useBuddyState(
  initialState?: BuddySemanticState,
  onBuddyEvent?: (event: BuddyEvent) => void,
): BuddyStateHandle {
  const [state, dispatch] = useReducer(
    (s: BuddySemanticState, a: SemanticAction) => reduceSemanticState(s, a),
    initialState ?? createInitialSemanticState(),
  );

  const reduxDispatch = useAppDispatch();
  const reduxSnapshot = useAppSelector(selectBuddySnapshot);

  const nowPlaying = useAppSelector(selectNowPlaying);
  const prevSnapshotProgressRef = useRef<{
    identityKey: string;
    stage: number;
  } | null>(null);
  const prevNowPlayingIdRef = useRef<string | null>(null);
  const prevLocalStageRef = useRef<number | null>(null);
  const prevLocalSkillsRef = useRef<string[] | null>(null);
  const onBuddyEventRef = useRef(onBuddyEvent);
  const [updateSettings] = useUpdateBuddySettingsMutation();
  useEffect(() => {
    onBuddyEventRef.current = onBuddyEvent;
  }, [onBuddyEvent]);

  useEffect(() => {
    if (!reduxSnapshot) return;
    const { identity } = reduxSnapshot.state;
    dispatch({
      kind: "patch",
      patch: {
        name: identity.name,
        paletteIndex: identity.palette_index,
      },
    });
  }, [reduxSnapshot]);

  useEffect(() => {
    if (!reduxSnapshot) return;
    const { personality, pet, semantic } = reduxSnapshot.state;
    dispatch({
      kind: "patch",
      patch: {
        personality: {
          playfulness: personality.traits.playfulness,
          confidence: Math.min(
            100,
            Math.round(
              (personality.traits.resilience +
                reduxSnapshot.state.progression.level * 8) /
                2,
            ),
          ),
          clinginess: clinginessFromAffection(pet.needs.affection),
          resilience: personality.traits.resilience,
          chaos: personality.traits.chaos,
          sociability: personality.traits.sociability,
          curiosity: personality.traits.curiosity,
        },
        mood: {
          happiness: Math.max(
            20,
            Math.round(
              (pet.needs.hunger + pet.needs.energy + pet.needs.affection) / 3,
            ),
          ),
          energy: pet.needs.energy,
          curiosity: personality.traits.curiosity,
          anxiety: anxietyFromNeglect(pet.evolution.neglect_score),
          boredom: pet.needs.boredom,
          affection: pet.needs.affection,
        },
        activity: {
          mood: semanticMood(semantic.mood, state.activity.mood),
          animationType:
            semantic.focus === "dreaming"
              ? "sleep"
              : semantic.focus === "play time"
                ? "perk"
                : semantic.focus === "helping"
                  ? "idle"
                  : state.activity.animationType,
          lastSignalTime: state.activity.lastSignalTime,
          lastSignalType: state.activity.lastSignalType,
        },
      },
    });
  }, [
    reduxSnapshot,
    state.activity.animationType,
    state.activity.lastSignalTime,
    state.activity.lastSignalType,
    state.activity.mood,
  ]);

  useEffect(() => {
    if (!reduxSnapshot) return;
    const { identity, progression } = reduxSnapshot.state;
    const curr = progression.stage;
    const identityKey = `${identity.name}:${identity.created_at}`;
    const prev = prevSnapshotProgressRef.current;
    const identityChanged = prev?.identityKey !== identityKey;
    prevSnapshotProgressRef.current = { identityKey, stage: curr };
    if (identityChanged) prevLocalStageRef.current = curr;

    dispatch({
      kind: "patch",
      patch: { progress: { xp: progression.xp, stage: curr } },
    });

    if (prev !== null && !identityChanged && curr > prev.stage) {
      dispatch({ kind: "signal", signalType: "stage_up" });
    }
  }, [reduxSnapshot]);

  const skillsKey = reduxSnapshot?.state.skills.unlocked.join(",") ?? "";
  useEffect(() => {
    if (!reduxSnapshot) return;
    dispatch({
      kind: "patch",
      patch: { skills: reduxSnapshot.state.skills.unlocked },
    });
  }, [reduxSnapshot, skillsKey]);

  useEffect(() => {
    const prev = prevLocalStageRef.current;
    const curr = state.progress.stage;
    prevLocalStageRef.current = curr;
    if (prev !== null && curr > prev) {
      const stageDef = STAGES[curr];
      onBuddyEventRef.current?.({
        type: "stage_evolved",
        stage: curr,
        name: stageDef.name,
      });
    }
  }, [state.progress.stage]);

  useEffect(() => {
    const prev = prevLocalSkillsRef.current;
    const curr = state.skills;
    prevLocalSkillsRef.current = curr;
    if (prev === null) return;
    const newSkills = curr.filter((s) => !prev.includes(s));
    for (const skillId of newSkills) {
      const def = SKILLS.find((s) => s.id === skillId);
      if (def) {
        onBuddyEventRef.current?.({
          type: "skill_unlocked",
          skillId: def.id,
          skillName: def.name,
        });
      }
    }
  }, [state.skills]);

  useEffect(() => {
    if (!nowPlaying) {
      prevNowPlayingIdRef.current = null;
      return;
    }
    const isNewEvent = nowPlaying.id !== prevNowPlayingIdRef.current;
    prevNowPlayingIdRef.current = nowPlaying.id;
    if (isNewEvent) {
      dispatch({ kind: "signal", signalType: nowPlaying.signal_type });
    }

    const signalDef = SIGNALS[nowPlaying.signal_type] as
      | (typeof SIGNALS)[keyof typeof SIGNALS]
      | undefined;
    const isActive = signalDef?.category === "active";
    const isCompleted =
      nowPlaying.status === "completed" || nowPlaying.status === "failed";
    if (isActive && !isCompleted) {
      return;
    }
    const ttl =
      nowPlaying.persistent && !isCompleted
        ? undefined
        : nowPlaying.ttl_ms ??
          signalDef?.duration ??
          (nowPlaying.status === "progress" ? 8000 : 4000);
    if (ttl === undefined) return;
    const timer = setTimeout(() => reduxDispatch(clearNowPlaying()), ttl);
    return () => clearTimeout(timer);
  }, [nowPlaying, reduxDispatch]);

  const signal = useCallback(
    (signalType: string) => dispatch({ kind: "signal", signalType }),
    [],
  );
  const addXP = useCallback(
    (amount: number) => dispatch({ kind: "add_xp", amount }),
    [],
  );
  const pet = useCallback(() => dispatch({ kind: "pet" }), []);
  const rename = useCallback(
    (name: string) => dispatch({ kind: "rename", name }),
    [],
  );
  const nextPalette = useCallback(() => {
    const nextIndex = (state.paletteIndex + 1) % 8;
    dispatch({ kind: "next_palette" });
    void updateSettings({ palette_index: nextIndex }).catch(() => undefined);
  }, [state.paletteIndex, updateSettings]);
  const reset = useCallback(() => dispatch({ kind: "reset" }), []);

  const handleCanvasEvent = useCallback((event: BuddyEvent) => {
    if (event.type === "xp_gained") {
      dispatch({ kind: "add_xp", amount: event.amount });
    } else if (event.type === "semantic_update") {
      dispatch({ kind: "patch", patch: event.patch });
    } else if (event.type === "petted") {
      dispatch({ kind: "pet" });
    }
  }, []);

  return {
    state,
    signal,
    addXP,
    pet,
    rename,
    nextPalette,
    reset,
    handleCanvasEvent,
    onBuddyEvent,
  };
}
