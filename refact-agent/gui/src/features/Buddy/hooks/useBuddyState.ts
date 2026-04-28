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
import type { BuddySemanticState, BuddyEvent } from "../types";

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
  const prevSnapshotStageRef = useRef<number | null>(null);
  const prevNowPlayingIdRef = useRef<string | null>(null);
  // null = not yet initialized; prevents fake milestone events on first hydration
  const prevLocalStageRef = useRef<number | null>(null);
  const prevLocalSkillsRef = useRef<string[] | null>(null);
  const onBuddyEventRef = useRef(onBuddyEvent);
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    reduxSnapshot?.state.identity.name,
    reduxSnapshot?.state.identity.palette_index,
  ]);

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
          clinginess: personality.traits.sociability,
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
          anxiety: Math.max(0, Math.round(pet.evolution.neglect_score / 2)),
          boredom: pet.needs.boredom,
          affection: pet.needs.affection,
        },
        activity: {
          mood:
            semantic.mood === "Sleepy"
              ? "sleepy"
              : semantic.mood === "Restless"
                ? "curious"
                : semantic.mood === "Questing"
                  ? "focused"
                  : state.activity.mood,
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    reduxSnapshot?.state.personality.traits.playfulness,
    reduxSnapshot?.state.personality.traits.chaos,
    reduxSnapshot?.state.personality.traits.sociability,
    reduxSnapshot?.state.personality.traits.curiosity,
    reduxSnapshot?.state.personality.traits.resilience,
    reduxSnapshot?.state.pet.needs.hunger,
    reduxSnapshot?.state.pet.needs.energy,
    reduxSnapshot?.state.pet.needs.boredom,
    reduxSnapshot?.state.pet.needs.affection,
    reduxSnapshot?.state.pet.evolution.neglect_score,
    reduxSnapshot?.state.semantic.mood,
    reduxSnapshot?.state.semantic.focus,
    reduxSnapshot?.state.progression.level,
  ]);

  useEffect(() => {
    if (!reduxSnapshot) return;
    const { progression } = reduxSnapshot.state;
    const curr = progression.stage;
    const prev = prevSnapshotStageRef.current;
    prevSnapshotStageRef.current = curr;

    dispatch({
      kind: "patch",
      patch: { progress: { xp: progression.xp, stage: curr } },
    });

    if (prev !== null && curr > prev) {
      dispatch({ kind: "signal", signalType: "stage_up" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    reduxSnapshot?.state.progression.stage,
    reduxSnapshot?.state.progression.xp,
  ]);

  // Skills sync is independent — fires when skills change even without XP/stage change
  const skillsKey = reduxSnapshot?.state.skills.unlocked.join(",") ?? "";
  useEffect(() => {
    if (!reduxSnapshot) return;
    dispatch({
      kind: "patch",
      patch: { skills: reduxSnapshot.state.skills.unlocked },
    });
    // skillsKey changes whenever unlocked array contents change
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [skillsKey]);

  // Emit stage_evolved and skill_unlocked events when canvas state changes.
  // prevLocalStageRef / prevLocalSkillsRef start as null so first hydration
  // from snapshot never triggers false milestone events.
  useEffect(() => {
    const prev = prevLocalStageRef.current;
    const curr = state.progress.stage;
    prevLocalStageRef.current = curr;
    // Only emit after the ref has been initialised (prev !== null)
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
    if (prev === null) return; // skip first hydration
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

  // Animation is driven solely by nowPlaying RuntimeEvents.
  // No signalQueue — RuntimeEvent is the single source of live Buddy UX.

  useEffect(() => {
    if (!nowPlaying) {
      prevNowPlayingIdRef.current = null;
      return;
    }
    // Only trigger animation burst when a genuinely NEW event arrives
    const isNewEvent = nowPlaying.id !== prevNowPlayingIdRef.current;
    prevNowPlayingIdRef.current = nowPlaying.id;
    if (isNewEvent) {
      dispatch({ kind: "signal", signalType: nowPlaying.signal_type });
    }

    // Backend may emit signal_type values not present in the local SIGNALS
    // map (e.g. newly-added or experimental types). Look up defensively so
    // an unknown type degrades gracefully instead of crashing the panel.
    const signalDef = SIGNALS[nowPlaying.signal_type] as
      | (typeof SIGNALS)[keyof typeof SIGNALS]
      | undefined;
    const isActive = signalDef?.category === "active";
    const isCompleted =
      nowPlaying.status === "completed" || nowPlaying.status === "failed";
    if (isActive && !isCompleted) {
      return;
    }
    const ttl = nowPlaying.persistent
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
  const nextPalette = useCallback(() => dispatch({ kind: "next_palette" }), []);
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
