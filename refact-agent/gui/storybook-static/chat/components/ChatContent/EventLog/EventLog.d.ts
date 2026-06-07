import React from "react";
import type { EventMessage } from "../../../services/refact/types";
export type EventLogProps = {
    events: EventMessage[];
    threadId: string;
    filterEvents?: EventMessage[];
    onProcessCompletedClick?: (processId: string) => void;
};
export declare const EventLog: React.FC<EventLogProps>;
