export type ConfigPatch = {
    path: (string | number)[];
    value: unknown;
};
export declare function applyPatch(obj: Record<string, unknown>, patch: ConfigPatch): Record<string, unknown>;
export declare function applyPatches(obj: Record<string, unknown>, patches: ConfigPatch[]): Record<string, unknown>;
export declare function getNestedValue<T>(obj: Record<string, unknown>, path: string[]): T | undefined;
export declare function isPlainObject(value: unknown): value is Record<string, unknown>;
export declare function sanitizeObject(obj: unknown): unknown;
export declare function extractSubagentExtra(config: Record<string, unknown>): Record<string, unknown>;
export declare function computeExtraPatches(oldExtra: Record<string, unknown>, newExtra: Record<string, unknown>): ConfigPatch[];
export declare function safeArray<T>(value: unknown, guard: (v: unknown) => v is T): T[];
export declare function safeString(value: unknown): string;
export declare function safeBoolean(value: unknown): boolean;
export declare function safeNumber(value: unknown): number | undefined;
export declare function safeObject(value: unknown): Record<string, unknown>;
export declare function isString(v: unknown): v is string;
export type MessageTemplate = {
    role: string;
    content: string;
};
export declare function isMessageTemplate(v: unknown): v is MessageTemplate;
export declare function safeMessageArray(value: unknown): MessageTemplate[];
export declare function safeSelectionRange(value: unknown): [number, number] | null;
export declare function parseIntSafe(value: string): number | undefined;
export declare function parseFloatSafe(value: string): number | undefined;
export declare function isToolConfirmRule(v: unknown): v is {
    match: string;
    action: string;
};
export declare function safeToolConfirmRules(value: unknown): {
    match: string;
    action: string;
}[];
export declare function validateConfigId(id: string): string | null;
