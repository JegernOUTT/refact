import { RefObject } from "react";
export declare function useDashboardResize(containerRef: RefObject<HTMLDivElement>, storageKey?: string, defaultRatio?: number): {
    ratio: number;
    handleDrag: (clientY: number) => void;
    reset: () => void;
};
