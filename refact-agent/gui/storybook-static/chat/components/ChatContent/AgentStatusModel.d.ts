export type AgentStatusState = "running" | "stuck" | "failed" | "done" | "paused";
export type AgentStatusTab = AgentStatusState | "all";
export type PriorityFilter = "all" | "P0" | "P1" | "P2";
export type AgeFilter = "all" | "15" | "60" | "240";
export type AgentAction = "pulse" | "diff" | "steer" | "pause" | "cancel";
export type AgentAlerts = {
    stuck: number;
    failed: number;
    paused: number;
};
export type AgentStatusRow = {
    priority: string;
    cardId: string;
    title: string;
    state: AgentStatusState;
    stateText: string;
    emoji: string;
    age: string;
    ageMinutes: number | null;
    lastTool: string | null;
    lastStatusUpdate: string | null;
    finalReport: string | null;
    raw: string;
};
export type AgentStatusReport = {
    alerts: AgentAlerts;
    rows: AgentStatusRow[];
    raw: string;
};
export type AgentStatusFilters = {
    tab: AgentStatusTab;
    priority: PriorityFilter;
    minAgeMinutes: number | null;
};
export declare const DEFAULT_CANCEL_REASON = "Cancelled from agent status view.";
export declare const DEFAULT_PAUSE_REASON = "Paused from agent pulse view.";
export declare const STATUS_TABS: AgentStatusTab[];
export declare function countAgentAlerts(rows: AgentStatusRow[]): AgentAlerts;
export declare function mergeAgentAlerts(primary: AgentAlerts, fallback: AgentAlerts): AgentAlerts;
export declare function parseAgentStatusOutput(content: string): AgentStatusReport | null;
export declare function filterAgentStatusRows(rows: AgentStatusRow[], filters: AgentStatusFilters): AgentStatusRow[];
export declare function formatAgentActionCommand(action: AgentAction, cardId: string, value?: string): string;
