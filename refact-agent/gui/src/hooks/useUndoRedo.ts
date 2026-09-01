import { useReducer } from "react";

export const MAX_HISTORY_ENTRIES = 100;
export const MAX_HISTORY_CHARS = 1_000_000;

const COALESCE_INTERVAL_MS = 400;
const COALESCE_MIN_LENGTH = 1_000;
const MAX_INCREMENTAL_CHANGE = 4;

enum ACTION_TYPES {
  SET_STATE = "SET_STATE",
  UNDO = "UNDO",
  REDO = "REDO",
  RESET = "RESET",
}

interface InternalState<T> {
  past: T[];
  present: T;
  future: T[];
  lastSetAt: number | null;
}

interface Action {
  type: ACTION_TYPES;
}

interface SetStateAction<T> extends Action {
  type: ACTION_TYPES.SET_STATE;
  payload: T;
  timestamp: number;
}

interface UndoAction extends Action {
  type: ACTION_TYPES.UNDO;
}

interface RedoAction extends Action {
  type: ACTION_TYPES.REDO;
}

interface ResetAction<T> extends Action {
  type: ACTION_TYPES.RESET;
  payload: T;
}

type Actions<T> = SetStateAction<T> | UndoAction | RedoAction | ResetAction<T>;

const retainedSize = <T>(value: T) =>
  typeof value === "string" ? value.length : 1;

const boundPast = <T>(values: T[]) => {
  let retainedChars = 0;
  let start = values.length;

  while (start > 0 && values.length - start < MAX_HISTORY_ENTRIES) {
    const nextSize = retainedSize(values[start - 1]);
    if (retainedChars + nextSize > MAX_HISTORY_CHARS) break;
    retainedChars += nextSize;
    start--;
  }

  if (start === values.length && values.length > 0) {
    start = values.length - 1;
  }

  return values.slice(start);
};

const boundFuture = <T>(values: T[]) => {
  let retainedChars = 0;
  let end = 0;

  while (end < values.length && end < MAX_HISTORY_ENTRIES) {
    const nextSize = retainedSize(values[end]);
    if (retainedChars + nextSize > MAX_HISTORY_CHARS) break;
    retainedChars += nextSize;
    end++;
  }

  return values.slice(0, end);
};

const isIncrementalTextChange = <T>(previous: T, next: T) => {
  if (typeof previous !== "string" || typeof next !== "string") return false;

  const growth = next.length - previous.length;
  const magnitude = Math.abs(growth);
  if (magnitude === 0 || magnitude > MAX_INCREMENTAL_CHANGE) return false;

  const [shorter, longer] = growth > 0 ? [previous, next] : [next, previous];
  return longer.startsWith(shorter);
};

const reducerWithUndoRedo = <T>(
  state: InternalState<T>,
  action: Actions<T>,
): InternalState<T> => {
  const { past, present, future } = state;

  switch (action.type) {
    case ACTION_TYPES.SET_STATE: {
      const shouldCoalesce =
        retainedSize(present) >= COALESCE_MIN_LENGTH &&
        state.lastSetAt !== null &&
        action.timestamp - state.lastSetAt <= COALESCE_INTERVAL_MS &&
        isIncrementalTextChange(present, action.payload);

      return {
        past: shouldCoalesce ? past : boundPast([...past, present]),
        present: action.payload,
        future: [],
        lastSetAt: action.timestamp,
      };
    }
    case ACTION_TYPES.UNDO: {
      if (past.length === 0) return state;
      return {
        past: past.slice(0, -1),
        present: past[past.length - 1],
        future: boundFuture([present, ...future]),
        lastSetAt: null,
      };
    }
    case ACTION_TYPES.REDO: {
      if (future.length === 0) return state;
      return {
        past: boundPast([...past, present]),
        present: future[0],
        future: future.slice(1),
        lastSetAt: null,
      };
    }
    case ACTION_TYPES.RESET: {
      return createInitialState(action.payload);
    }
    default: {
      return state;
    }
  }
};

const createInitialState = <T>(initialState: T): InternalState<T> => ({
  past: [],
  present: initialState,
  future: [],
  lastSetAt: null,
});

export const useUndoRedo = <T>(initialState: T) => {
  const [state, dispatch] = useReducer(
    reducerWithUndoRedo<T>,
    createInitialState(initialState),
  );
  const { past, present, future } = state;

  const setState = (newState: T) =>
    dispatch({
      type: ACTION_TYPES.SET_STATE,
      payload: newState,
      timestamp: Date.now(),
    });

  const isUndoPossible = past.length > 0;
  const undo = () => {
    if (isUndoPossible) dispatch({ type: ACTION_TYPES.UNDO });
  };

  const isRedoPossible = future.length > 0;
  const redo = () => {
    if (isRedoPossible) dispatch({ type: ACTION_TYPES.REDO });
  };

  const reset = (payload: T) => dispatch({ type: ACTION_TYPES.RESET, payload });

  return {
    state: present,
    setState,
    undo,
    redo,
    reset,
    pastStates: past,
    futureStates: future,
    isUndoPossible,
    isRedoPossible,
  };
};
