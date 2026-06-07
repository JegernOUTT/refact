import React from "react";
export type LogoAnimationProps = {
    isWaiting: boolean;
    isStreaming: boolean;
    size?: "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9";
};
export declare const LogoAnimation: React.FC<LogoAnimationProps>;
