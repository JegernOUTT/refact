import React from "react";
type Segment = {
    value: number;
    color: string;
    label: string;
};
type MiniDonutProps = {
    segments: Segment[];
    size?: number;
    strokeWidth?: number;
};
export declare const MiniDonut: React.FC<MiniDonutProps>;
export {};
