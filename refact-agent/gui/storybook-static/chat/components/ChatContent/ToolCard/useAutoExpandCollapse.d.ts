export type ToolStatus = "running" | "success" | "error";
interface UseAutoExpandCollapseOptions {
    status: ToolStatus;
    collapseDelayMs?: number;
    storeKey?: string;
}
interface UseAutoExpandCollapseResult {
    isOpen: boolean;
    onToggle: () => void;
    animate: boolean;
}
export declare function useAutoExpandCollapse({ status, collapseDelayMs, storeKey, }: UseAutoExpandCollapseOptions): UseAutoExpandCollapseResult;
export {};
