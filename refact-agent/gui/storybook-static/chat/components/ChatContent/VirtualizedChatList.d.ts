import { JSX } from 'react/jsx-runtime';
import React from "react";
export type VirtualizedChatListProps<T extends {
    key: string;
}> = {
    items: T[];
    renderItem: (item: T) => React.ReactNode;
    initialScrollIndex?: number;
    footer?: React.ReactNode;
    header?: React.ReactNode;
    isStreaming?: boolean;
};
export declare function VirtualizedChatList<T extends {
    key: string;
}>({ items, renderItem, initialScrollIndex, footer, header, isStreaming, }: VirtualizedChatListProps<T>): JSX.Element;
export declare namespace VirtualizedChatList {
    var displayName: string;
}
