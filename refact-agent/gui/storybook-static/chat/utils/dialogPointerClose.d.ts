import type { HTMLAttributes, SyntheticEvent } from "react";
type DialogCloseHandlers = Pick<HTMLAttributes<HTMLElement>, "onPointerDownCapture" | "onMouseDownCapture" | "onClickCapture">;
export declare function closeDialogOnNonInteractiveEvent(event: SyntheticEvent<HTMLElement>, close: () => void): void;
export declare function dialogNonInteractiveCloseHandlers(close: () => void): DialogCloseHandlers;
export {};
