export declare function useCollapsibleState(defaultOpen?: boolean): {
    isOpen: (key: string) => boolean;
    setOpen: (key: string, open: boolean) => void;
    toggle: (key: string) => void;
    reset: () => void;
};
export type CollapsibleStateManager = ReturnType<typeof useCollapsibleState>;
