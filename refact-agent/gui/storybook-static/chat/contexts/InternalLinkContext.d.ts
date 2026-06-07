import React from "react";
export type InternalLinkHandler = (url: string) => boolean;
export interface InternalLinkContextValue {
    handleInternalLink: InternalLinkHandler;
}
export declare const InternalLinkContext: React.Context<InternalLinkContextValue | null>;
interface InternalLinkProviderProps {
    onInternalLink: InternalLinkHandler;
    children: React.ReactNode;
}
export declare const InternalLinkProvider: React.FC<InternalLinkProviderProps>;
export {};
