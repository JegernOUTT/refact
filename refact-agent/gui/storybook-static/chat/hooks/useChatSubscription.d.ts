import { type ChatEventEnvelope } from "../services/refact/chatSubscription";
export type ConnectionStatus = "disconnected" | "connecting" | "connected";
export type UseChatSubscriptionOptions = {
    /** Enable subscription (default: true) */
    enabled?: boolean;
    /** Reconnect on error (default: true) */
    autoReconnect?: boolean;
    /** Reconnect delay in ms (default: 2000) */
    reconnectDelay?: number;
    /** Callback when event received */
    onEvent?: (event: ChatEventEnvelope) => void;
    /** Callback when connected */
    onConnected?: () => void;
    /** Callback when disconnected */
    onDisconnected?: () => void;
    /** Callback when error occurs */
    onError?: (error: Error) => void;
};
/**
 * Hook for subscribing to chat events via SSE.
 *
 * @param chatId - Chat ID to subscribe to
 * @param options - Configuration options
 * @returns Connection status and control functions
 */
export declare function useChatSubscription(chatId: string | null | undefined, options?: UseChatSubscriptionOptions): {
    status: ConnectionStatus;
    error: Error | null;
    lastSeq: string;
    connect: () => void;
    disconnect: () => void;
    reconnect: () => void;
    isConnected: boolean;
    isConnecting: boolean;
};
export default useChatSubscription;
