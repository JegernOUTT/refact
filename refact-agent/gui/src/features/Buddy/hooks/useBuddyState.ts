import { useCallback, useEffect, useReducer } from "react";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  createInitialSemanticState,
  reduceSemanticState,
  type SemanticAction,
} from "../state";
import {
  selectBuddySnapshot,
  selectBuddySignalQueue,
  consumeBuddySignal,
} from "../buddySlice";
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

  const reduxDispatch = useAppDispatch();
  const reduxSnapshot = useAppSelector(selectBuddySnapshot);
  const signalQueue = useAppSelector(selectBuddySignalQueue);

  useEffect(() => {
    if (!reduxSnapshot) return;
    const { identity } = reduxSnapshot.state;
    dispatch({
      kind: "patch",
      patch: {
        name: identity.name,
        paletteIndex: reduxSnapshot.settings.palette_index,
      },
    });
  }, [reduxSnapshot?.state.identity.name, reduxSnapshot?.settings.palette_index]);

  useEffect(() => {
    if (signalQueue.length === 0) return;
    const next = signalQueue[0];
    dispatch({ kind: "signal", signalType: next.signalType });
    reduxDispatch(consumeBuddySignal());
  }, [signalQueue, reduxDispatch]);

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
