import { current, isDraft, type Draft } from "@reduxjs/toolkit";
import type { Chat, ChatThreadRuntime } from "./types";
import {
  getChatActivity,
  type ChatActivity,
} from "../../../utils/chatActivity";
import type { BackgroundAgentSummary } from "../../../services/refact/types";

export type ThreadDisplayMetadata = {
  title?: string;
  mode?: string;
  session_state?: string;
  activity: ChatActivity;
  is_buddy_chat: boolean;
  is_task_chat: boolean;
};
export type ThreadMetadata = {
  ids: string[];
  display: Record<string, ThreadDisplayMetadata | undefined>;
  agents: Record<string, Record<string, BackgroundAgentSummary> | undefined>;
};

export function displayMetadata(rt: ChatThreadRuntime): ThreadDisplayMetadata {
  return {
    title: rt.thread.title,
    mode: rt.thread.mode,
    session_state:
      (rt.session_state === "" ? undefined : rt.session_state) ??
      (rt.error
        ? "error"
        : rt.confirmation.pause
          ? "paused"
          : rt.streaming
            ? "generating"
            : rt.waiting_for_response
              ? "executing_tools"
              : "idle"),
    activity: getChatActivity(rt),
    is_buddy_chat: Boolean(rt.thread.buddy_meta?.is_buddy_chat),
    is_task_chat: Boolean(rt.thread.is_task_chat),
  };
}

export function createThreadMetadata(threads: Chat["threads"]): ThreadMetadata {
  const ids = Object.keys(threads).filter((id) => threads[id] !== undefined);
  const display: ThreadMetadata["display"] = {};
  const agents: ThreadMetadata["agents"] = {};
  for (const id of ids) {
    const rt = threads[id];
    if (!rt) continue;
    display[id] = displayMetadata(rt);
    agents[id] = rt.background_agents;
  }
  return { ids, display, agents };
}

export function refreshThreadMetadata(state: Draft<Chat>, targetId?: string) {
  if (!state.metadata) {
    state.metadata = createThreadMetadata(state.threads);
    return;
  }
  const metadata = state.metadata;
  const ids =
    targetId === undefined
      ? Object.keys(state.threads).filter(
          (id) => state.threads[id] !== undefined,
        )
      : undefined;
  if (ids) {
    if (
      ids.length !== metadata.ids.length ||
      ids.some((id, i) => id !== metadata.ids[i])
    )
      metadata.ids = ids;
  } else if (
    targetId !== undefined &&
    Boolean(state.threads[targetId]) !== Boolean(metadata.display[targetId])
  ) {
    metadata.ids = Object.keys(state.threads).filter(
      (id) => state.threads[id] !== undefined,
    );
  }
  const targets =
    targetId === undefined
      ? new Set([...Object.keys(metadata.display), ...metadata.ids])
      : [targetId];
  for (const id of targets) {
    const rt = state.threads[id];
    if (!rt) {
      Reflect.deleteProperty(metadata.display, id);
      Reflect.deleteProperty(metadata.agents, id);
      continue;
    }
    const next = displayMetadata(rt);
    const previous = metadata.display[id];
    if (
      !previous ||
      previous.title !== next.title ||
      previous.mode !== next.mode ||
      previous.session_state !== next.session_state ||
      previous.activity !== next.activity ||
      previous.is_buddy_chat !== next.is_buddy_chat ||
      previous.is_task_chat !== next.is_task_chat
    )
      metadata.display[id] = next;
    const agents = isDraft(rt.background_agents)
      ? current(rt.background_agents)
      : rt.background_agents;
    const previousAgents = metadata.agents[id];
    const previousSource =
      previousAgents && isDraft(previousAgents)
        ? current(previousAgents)
        : previousAgents;
    if (previousSource !== agents) metadata.agents[id] = agents;
  }
}
