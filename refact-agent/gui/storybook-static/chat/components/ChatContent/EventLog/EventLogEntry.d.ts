import React from "react";
import type { EventMessage } from "../../../services/refact/types";
type EventLogEntryProps = {
    event: EventMessage;
    entryId: string;
    onEventClick?: (event: EventMessage) => boolean;
};
export declare const EventLogEntry: React.FC<EventLogEntryProps>;
export {};
