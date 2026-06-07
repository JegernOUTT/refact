import React from "react";
import { DiffMessage, type DiffChunk } from "../../services/refact";
type DiffType = "apply" | "unapply" | "error" | "can not apply";
type DiffProps = {
    diff: DiffChunk;
};
export declare const Diff: React.FC<DiffProps>;
export type DiffChunkWithTypeAndApply = DiffChunk & {
    type: DiffType;
    apply: boolean;
};
export declare const DiffTitle: React.FC<{
    diffs: Record<string, DiffChunk[]>;
}>;
export declare const DiffContent: React.FC<{
    diffs: Record<string, DiffChunk[]>;
    open?: boolean;
    onOpenChange?: (open: boolean) => void;
}>;
export type DiffWithStatus = DiffChunk & {
    state?: 0 | 1 | 2;
    can_apply: boolean;
    applied: boolean;
    index: number;
};
export declare const DiffForm: React.FC<{
    diffs: Record<string, DiffChunk[]>;
}>;
type GroupedDiffsProps = {
    diffs: DiffMessage[];
    open?: boolean;
    onOpenChange?: (open: boolean) => void;
};
export declare const GroupedDiffs: React.NamedExoticComponent<GroupedDiffsProps>;
export {};
