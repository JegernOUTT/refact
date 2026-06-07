import { type EngineApiConfig } from "./apiUrl";
export type EngineApiConnection = EngineApiConfig;
export type PortOrConnection = number | EngineApiConnection;
export declare function normalizeConnection(input: PortOrConnection): EngineApiConfig;
export type MessageContent = string | ({
    type: "text";
    text: string;
} | {
    type: "image_url";
    image_url: {
        url: string;
    };
})[];
export type ChatCommandBase = {
    type: "user_message";
    content: MessageContent;
    attachments?: unknown[];
} | {
    type: "retry_from_index";
    index: number;
    content?: MessageContent;
    attachments?: unknown[];
} | {
    type: "set_params";
    patch: Record<string, unknown>;
} | {
    type: "abort";
} | {
    type: "tool_decision";
    tool_call_id: string;
    accepted: boolean;
} | {
    type: "tool_decisions";
    decisions: {
        tool_call_id: string;
        accepted: boolean;
    }[];
} | {
    type: "ide_tool_result";
    tool_call_id: string;
    content: string;
    tool_failed: boolean;
} | {
    type: "update_message";
    message_id: string;
    content: MessageContent;
    attachments?: unknown[];
    regenerate?: boolean;
} | {
    type: "remove_message";
    message_id: string;
    regenerate?: boolean;
} | {
    type: "regenerate";
} | {
    type: "branch_from_chat";
    source_chat_id: string;
    up_to_message_id: string;
} | {
    type: "browser_context_decision";
    pending_message_id: string;
    include_actions: boolean;
    include_console: boolean;
    include_network: boolean;
    include_mutations: boolean;
    include_screenshot: boolean;
    last_n_actions?: number | null;
    last_n_console?: number | null;
    last_n_network?: number | null;
};
export type ChatCommand = ChatCommandBase & {
    client_request_id: string;
    priority?: boolean;
};
export declare function sendChatCommand(chatId: string, connection: PortOrConnection, apiKey: string | undefined, command: ChatCommandBase, priority?: boolean): Promise<void>;
export declare function sendUserMessage(chatId: string, content: MessageContent, connection: PortOrConnection, apiKey?: string, priority?: boolean, contextFiles?: unknown[], suppressAutoEnrichment?: boolean): Promise<void>;
export declare function retryFromIndex(chatId: string, index: number, content: MessageContent, connection: PortOrConnection, apiKey?: string): Promise<void>;
export declare function regenerate(chatId: string, connection: PortOrConnection, apiKey?: string): Promise<void>;
export declare function updateChatParams(chatId: string, params: Record<string, unknown>, connection: PortOrConnection, apiKey?: string): Promise<void>;
export declare function abortGeneration(chatId: string, connection: PortOrConnection, apiKey?: string): Promise<void>;
export declare function respondToToolConfirmation(chatId: string, toolCallId: string, accepted: boolean, connection: PortOrConnection, apiKey?: string): Promise<void>;
export declare function respondToToolConfirmations(chatId: string, decisions: {
    tool_call_id: string;
    accepted: boolean;
}[], connection: PortOrConnection, apiKey?: string): Promise<void>;
export declare function updateMessage(chatId: string, messageId: string, content: MessageContent, connection: PortOrConnection, apiKey?: string, regenerate?: boolean): Promise<void>;
export declare function removeMessage(chatId: string, messageId: string, connection: PortOrConnection, apiKey?: string, regenerate?: boolean): Promise<void>;
export declare function branchFromChat(targetChatId: string, sourceChatId: string, upToMessageId: string, connection: PortOrConnection, apiKey?: string): Promise<void>;
export declare function sendBrowserContextDecision(chatId: string, connection: PortOrConnection, decision: {
    pending_message_id: string;
    include_actions: boolean;
    include_console: boolean;
    include_network: boolean;
    include_mutations: boolean;
    include_screenshot: boolean;
    last_n_actions?: number | null;
    last_n_console?: number | null;
    last_n_network?: number | null;
}, apiKey?: string): Promise<void>;
export declare function cancelQueuedItem(chatId: string, clientRequestId: string, connection: PortOrConnection, apiKey?: string): Promise<boolean>;
