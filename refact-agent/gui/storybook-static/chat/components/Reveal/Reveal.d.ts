import React from "react";
export type RevealProps = {
    children: React.ReactNode;
    defaultOpen: boolean;
    isRevealingCode?: boolean;
    onClose?: () => void;
    storeKey?: string;
};
export declare const Reveal: React.FC<RevealProps>;
