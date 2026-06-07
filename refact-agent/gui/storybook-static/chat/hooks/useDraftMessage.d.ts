export declare function useDraftMessage(): {
    value: string;
    setValue: (newValue: string | ((prev: string) => string)) => void;
    clearDraft: () => void;
};
