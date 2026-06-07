import { type ChatCommandBase } from "../services/refact/chatCommands";
export declare function useSendChatCommand(): (chatId: string, command: ChatCommandBase) => Promise<void>;
