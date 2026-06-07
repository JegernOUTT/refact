import React from "react";
type AboutFunction = (reason?: string) => void;
type AbortControllerContext = {
    addAbortController: (key: string, fn: AboutFunction) => void;
    abort: (key: string, reason?: string) => void;
    removeController: (key: string) => void;
};
export declare const AbortControllerContext: React.Context<AbortControllerContext | null>;
export declare const AbortControllerProvider: React.FC<{
    children: React.ReactNode;
}>;
export {};
