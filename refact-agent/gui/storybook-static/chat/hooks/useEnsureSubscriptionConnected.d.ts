export declare function useEnsureSubscriptionConnected(chatId: string | null | undefined): {
    ensureConnected: () => Promise<void>;
    isConnected: boolean;
    isConnecting: boolean;
};
