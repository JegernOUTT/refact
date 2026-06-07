import { ActionCreatorWithPayload } from '@reduxjs/toolkit';
export declare const useAppearance: () => {
    appearance: "light" | "inherit" | "dark" | undefined;
    setAppearance: ActionCreatorWithPayload<"light" | "inherit" | "dark", string>;
    isDarkMode: boolean;
    toggle: () => {
        payload: "light" | "inherit" | "dark";
        type: string;
    } | undefined;
};
