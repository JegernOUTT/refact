type PickDesiredChatSubscriptionsArgs = {
    openThreadIds: string[];
    activeChatId: string | null | undefined;
    subscribedThreadIds: string[];
    maxSubscriptions?: number;
};
export declare function pickDesiredChatSubscriptions({ openThreadIds, activeChatId, subscribedThreadIds, maxSubscriptions, }: PickDesiredChatSubscriptionsArgs): string[];
export declare function useAllChatsSubscription(): void;
export {};
