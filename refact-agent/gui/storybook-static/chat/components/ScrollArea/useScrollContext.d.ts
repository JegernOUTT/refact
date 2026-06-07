import { Context, type RefObject } from 'react';
type State = {
    innerRef: RefObject<HTMLDivElement> | null;
    scrollRef: RefObject<HTMLDivElement> | null;
    anchorRef: RefObject<HTMLDivElement> | null;
    bottomRef: RefObject<HTMLDivElement> | null;
    anchorProps: ScrollIntoViewOptions | null;
    scrolled: boolean;
    mode: "user-message" | "manual" | "follow";
};
type Action = {
    type: "set_anchor";
    payload: RefObject<HTMLDivElement> | null;
} | {
    type: "set_bottom";
    payload: RefObject<HTMLDivElement> | null;
} | {
    type: "upsert_refs";
    payload: Partial<State>;
} | {
    type: "set_anchor_props";
    payload: ScrollIntoViewOptions | null;
} | {
    type: "set_scrolled";
    payload: boolean;
} | {
    type: "set_mode";
    payload: State["mode"];
};
type Dispatch = (action: Action) => void;
export declare const ScrollAreaWithAnchorContext: Context<{
    state: State;
    dispatch: Dispatch;
} | null>;
export declare function useScrollContext(): {
    state: State;
    dispatch: Dispatch;
};
export declare function scrollAreaWithAnchorReducer(state: State, action: Action): State | {
    anchor_props: ScrollIntoViewOptions | null;
    innerRef: RefObject<HTMLDivElement> | null;
    scrollRef: RefObject<HTMLDivElement> | null;
    anchorRef: RefObject<HTMLDivElement> | null;
    bottomRef: RefObject<HTMLDivElement> | null;
    anchorProps: ScrollIntoViewOptions | null;
    scrolled: boolean;
    mode: "user-message" | "manual" | "follow";
};
export {};
