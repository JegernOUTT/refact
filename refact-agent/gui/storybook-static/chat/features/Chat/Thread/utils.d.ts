import { ChatMessages, ToolCall, ThinkingBlock } from "../../../services/refact";
import { type LspChatMessage } from "../../../services/refact";
export declare function postProcessMessagesAfterStreaming(messages: ChatMessages): ChatMessages;
export declare const TAKE_NOTE_MESSAGE = "How many times did you used a tool incorrectly, so it didn't produce the indented result? Call remember_how_to_use_tools() with this exact format:\n\nCORRECTION_POINTS: N\n\nPOINT1 WHAT_I_DID_WRONG: i should have used ... tool call or method or plan ... instead of this tool call or method or plan.\nPOINT1 FOR_FUTURE_FEREFENCE: when ... [describe situation when it's applicable] use ... tool call or method or plan.\n\nPOINT2 WHAT_I_DID_WRONG: ...\nPOINT2 FOR_FUTURE_FEREFENCE: ...\n";
export declare function mergeToolCalls(prev: ToolCall[], add: ToolCall[]): ToolCall[];
export declare function mergeThinkingBlocks(prev: ThinkingBlock[], add: ThinkingBlock[]): ThinkingBlock[];
export declare function lastIndexOf<T>(arr: T[], predicate: (a: T) => boolean): number;
export declare function formatMessagesForLsp(messages: ChatMessages): LspChatMessage[];
export declare function formatMessagesForChat(messages: LspChatMessage[]): ChatMessages;
