export declare const useUndoRedo: <T>(initialState: T) => {
    state: T;
    setState: (newState: T) => void;
    undo: () => void;
    redo: () => void;
    reset: (payload: T) => void;
    pastStates: T[];
    futureStates: T[];
    isUndoPossible: boolean;
    isRedoPossible: boolean;
};
