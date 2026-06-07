export type AgentPulseState = "running" | "paused" | "waiting" | "done" | "error" | "idle" | "unknown";
export type AgentPulseReport = {
    cardId: string;
    cardTitle: string;
    state: string;
    stateKind: AgentPulseState;
    lastActivity: string;
    tokens: string;
    currentlyEditing: string;
    lastAssistantMessage: string;
    lastToolCall: string;
    sessionNote: string | null;
    raw: string;
};
export declare function parseAgentPulseOutput(content: string): AgentPulseReport | null;
