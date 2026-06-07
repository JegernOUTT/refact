export declare const DOCUMENT_KINDS: readonly ["plan", "design", "runbook", "brief", "postmortem", "spec"];
export type KnownDocumentKind = (typeof DOCUMENT_KINDS)[number];
export declare const MEMORY_KINDS: readonly ["decision", "spec", "finding", "gotcha", "risk", "handoff", "progress", "postmortem", "brief", "freeform"];
export type KnownMemoryKind = (typeof MEMORY_KINDS)[number];
type BadgeColor = "blue" | "purple" | "green" | "teal" | "red" | "gray" | "amber";
export declare function documentKindColor(kind: string): BadgeColor;
export declare function memoryKindColor(kind: string): BadgeColor;
export {};
