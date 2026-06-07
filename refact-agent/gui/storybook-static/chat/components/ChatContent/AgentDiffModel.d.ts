import type { DiffChunk } from "../../services/refact/types";
type DiffMode = "unified" | "stat" | "name-only";
type DiffStats = {
    files: number;
    added: number;
    removed: number;
};
export type AgentDiffReport = {
    cardId: string;
    cardTitle: string;
    branch: string;
    base: string;
    mode: DiffMode;
    body: string;
    files: string[];
    stats: DiffStats;
    truncated: string | null;
    diffChunks: DiffChunk[];
    raw: string;
};
export declare function parseAgentDiffOutput(content: string): AgentDiffReport | null;
export {};
