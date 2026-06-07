import { type MessageContent } from "../services/refact/chatCommands";
import type { UserMessage } from "../services/refact/types";
export declare function useChatActions(): {
    submit: (question: string, priority?: boolean) => Promise<void>;
    abort: () => Promise<void>;
    setParams: (params: {
        model?: string;
        mode?: string;
        boost_reasoning?: boolean;
    }) => Promise<void>;
    respondToTool: (toolCallId: string, accepted: boolean) => Promise<void>;
    respondToTools: (decisions: {
        tool_call_id: string;
        accepted: boolean;
    }[]) => Promise<void>;
    retryFromIndex: (index: number, newContent: UserMessage["content"]) => Promise<void>;
    updateMessage: (messageId: string, newContent: MessageContent, regenerate?: boolean) => Promise<void>;
    removeMessage: (messageId: string, regenerate?: boolean) => Promise<void>;
    regenerate: () => Promise<void>;
    cancelQueued: (clientRequestId: string) => Promise<boolean>;
};
export default useChatActions;
