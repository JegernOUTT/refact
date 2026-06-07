type CollapseState = {
    buddy: boolean;
    chats: boolean;
    tasks: boolean;
};
export declare function useDashboardCollapseState(): {
    collapsed: CollapseState;
    toggle: (key: keyof CollapseState) => void;
};
export {};
