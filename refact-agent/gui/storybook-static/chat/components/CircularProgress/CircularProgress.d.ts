import React from "react";
export interface CircularProgressProps {
    done: number;
    total: number;
    failed?: number;
    size?: number;
}
export declare const CircularProgress: React.FC<CircularProgressProps>;
