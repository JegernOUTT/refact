import { useCallback, useReducer } from "react";
import {
  createInitialSemanticState,
  reduceSemanticState,
  type SemanticAction,
} from "../state";
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
}

export function useBuddyState(
  initialState?: BuddySemanticState,
): BuddyStateHandle {
  const [state, dispatch] = useReducer(
    (s: BuddySemanticState, a: SemanticAction) => reduceSemanticState(s, a),
    initialState ?? createInitialSemanticState(),
  );

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
  };
}
