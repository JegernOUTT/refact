import type { TrajectoryMeta, TrajectoryEvent } from "./trajectories";
import type { TaskMeta, TaskBoard } from "./tasks";
import type { BuddySnapshot, BuddySSEEvent } from "../../features/Buddy/types";
import { type EngineApiConfig } from "./apiUrl";
export type { TrajectoryMeta, TrajectoryEvent };
export type TaskEvent = {
    type: "snapshot";
    tasks: TaskMeta[];
} | {
    type: "task_created";
    task_id: string;
    meta: TaskMeta;
} | {
    type: "task_updated";
    task_id: string;
    meta: TaskMeta;
} | {
    type: "task_deleted";
    task_id: string;
} | {
    type: "board_changed";
    task_id: string;
    rev: number;
    board: TaskBoard;
};
export type NotificationEvent = {
    type: "task_done";
    chat_id: string;
    tool_call_id: string;
    summary: string;
    knowledge_path?: string;
} | {
    type: "ask_questions";
    chat_id: string;
    tool_call_id: string;
    questions: {
        id: string;
        type: string;
        text: string;
        options?: string[];
    }[];
};
export type SidebarSection = "workspace" | "chats" | "tasks" | "buddy";
export type SidebarSectionStatus = "ready" | "error";
export type BuddySnapshotPayload = BuddySnapshot | null;
export type SidebarPagination = {
    next_cursor: string | null;
    has_more: boolean;
    total_count: number;
};
export type SidebarSectionSnapshot = {
    workspace_roots: string[];
} | {
    trajectories: TrajectoryMeta[];
    pagination?: SidebarPagination;
} | {
    tasks: TaskMeta[];
} | {
    buddy: BuddySnapshotPayload;
};
export type SidebarSectionUpdate = TrajectoryEvent | TaskEvent | BuddySSEEvent;
export type SidebarKnownEvent = {
    type: "section_snapshot";
    section: SidebarSection;
    status: SidebarSectionStatus;
    snapshot: SidebarSectionSnapshot;
    elapsed_ms?: number;
    error?: string;
} | {
    type: "section_update";
    section: SidebarSection;
    update: SidebarSectionUpdate;
} | {
    type: "notification";
    notification: NotificationEvent;
} | {
    type: "heartbeat";
    payload: {
        ts: string;
    };
};
export type SidebarEvent = SidebarKnownEvent | {
    type: string;
    payload: unknown;
};
export type SidebarEventEnvelope = {
    protocol_version: 2;
    seq: number;
    subscription_id: string;
    event: SidebarEvent;
};
export type SidebarDispatchedEvent = Exclude<SidebarKnownEvent, {
    type: "heartbeat";
}>;
export type SidebarDispatchedEventEnvelope = Omit<SidebarEventEnvelope, "event"> & {
    event: SidebarDispatchedEvent;
};
export type SidebarSubscriptionCallbacks = {
    onEvent: (event: SidebarDispatchedEventEnvelope) => void;
    onError: (error: Error) => void;
    onConnected?: () => void;
    onDisconnected?: () => void;
    onLiveness?: () => void;
};
export declare function subscribeToSidebarEvents(config: EngineApiConfig, apiKey: string | null, callbacks: SidebarSubscriptionCallbacks): () => void;
