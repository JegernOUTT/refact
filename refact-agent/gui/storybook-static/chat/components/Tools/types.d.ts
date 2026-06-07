import { ToolCall } from "../../services/refact";
export declare const TEXTDOC_TOOL_NAMES: string[];
type TextDocToolNames = (typeof TEXTDOC_TOOL_NAMES)[number];
export interface RawTextDocTool extends ToolCall {
    function: {
        name: TextDocToolNames;
        arguments: string;
    };
}
export declare const isRawTextDocToolCall: (toolCall: ToolCall) => toolCall is RawTextDocTool;
export type ParsedRawTextDocToolCall = Omit<RawTextDocTool, "function"> & {
    function: {
        name: TextDocToolNames;
        arguments: Record<string, string | boolean | number | undefined>;
    };
};
export declare const isParseRawTextDocToolCall: (json: unknown) => json is ParsedRawTextDocToolCall;
export interface CreateTextDocToolCall extends ParsedRawTextDocToolCall {
    function: {
        name: "create_textdoc";
        arguments: {
            path: string;
            content: string;
        };
    };
}
export declare const isCreateTextDocToolCall: (toolCall: ParsedRawTextDocToolCall) => toolCall is CreateTextDocToolCall;
export interface UpdateTextDocToolCall extends ParsedRawTextDocToolCall {
    function: {
        name: "update_textdoc";
        arguments: {
            path: string;
            old_str: string;
            replacement: string;
            multiple: boolean;
        };
    };
}
export declare const isUpdateTextDocToolCall: (toolCall: ParsedRawTextDocToolCall) => toolCall is UpdateTextDocToolCall;
export interface UpdateRegexTextDocToolCall extends ParsedRawTextDocToolCall {
    function: {
        name: string;
        arguments: {
            path: string;
            pattern: string;
            replacement: string;
            multiple: boolean;
        };
    };
}
export declare const isUpdateRegexTextDocToolCall: (toolCall: ParsedRawTextDocToolCall) => toolCall is UpdateRegexTextDocToolCall;
export interface ReplaceTextDocToolCall extends ParsedRawTextDocToolCall {
    function: {
        name: string;
        arguments: {
            path: string;
            replacement: string;
        };
    };
}
export declare const isReplaceTextDocToolCall: (toolCall: ParsedRawTextDocToolCall) => toolCall is ReplaceTextDocToolCall;
export interface UpdateTextDocByLinesToolCall extends ParsedRawTextDocToolCall {
    function: {
        name: string;
        arguments: {
            path: string;
            content: string;
            ranges: string;
        };
    };
}
export declare const isUpdateTextDocByLinesToolCall: (toolCall: ParsedRawTextDocToolCall) => toolCall is UpdateTextDocByLinesToolCall;
export interface UpdateTextDocAnchoredToolCall extends ParsedRawTextDocToolCall {
    function: {
        name: "update_textdoc_anchored";
        arguments: {
            path: string;
            anchor1: string;
            anchor2?: string;
            content: string;
            mode: "replace_between" | "insert_after" | "insert_before";
            multiple?: boolean;
        };
    };
}
export declare const isUpdateTextDocAnchoredToolCall: (toolCall: ParsedRawTextDocToolCall) => toolCall is UpdateTextDocAnchoredToolCall;
export interface ApplyPatchToolCall extends ParsedRawTextDocToolCall {
    function: {
        name: "apply_patch";
        arguments: {
            patch: string;
        };
    };
}
export declare const isApplyPatchToolCall: (toolCall: ParsedRawTextDocToolCall) => toolCall is ApplyPatchToolCall;
export interface UndoTextDocToolCall extends ParsedRawTextDocToolCall {
    function: {
        name: "undo_textdoc";
        arguments: {
            path: string;
            steps?: number;
        };
    };
}
export declare const isUndoTextDocToolCall: (toolCall: ParsedRawTextDocToolCall) => toolCall is UndoTextDocToolCall;
export type TextDocToolCall = CreateTextDocToolCall | UpdateTextDocToolCall | ReplaceTextDocToolCall | UpdateRegexTextDocToolCall | UpdateTextDocByLinesToolCall | UpdateTextDocAnchoredToolCall | ApplyPatchToolCall | UndoTextDocToolCall;
export declare function parseRawTextDocToolCall(toolCall: RawTextDocTool): TextDocToolCall | null;
export {};
