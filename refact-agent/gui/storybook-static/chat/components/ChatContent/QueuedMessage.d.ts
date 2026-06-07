import React from "react";
import type { QueuedItem } from "../../features/Chat";
type QueuedMessageProps = {
    queuedItem: QueuedItem;
    position: number;
};
export declare const QueuedMessage: React.FC<QueuedMessageProps>;
export default QueuedMessage;
