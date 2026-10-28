import React from "react";
import { renderHook, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import { Provider } from "react-redux";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../app/store";
import { useSkillsStatus } from "./useSkillsStatus";
import { server } from "../utils/mockServer";
import { createDefaultChatState } from "../utils/test-utils";
import { applyChatEvent } from "../features/Chat/Thread/actions";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";

const CHAT_ID = "chat-skills";

const LOADED_SKILLS = {
  skills_available: 2,
  skills_included: ["review"],
  skills_enabled: true,
  active_skill: "review",
};

function renderSkillsStatus() {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.id = CHAT_ID;
  runtime.last_applied_seq = "1";
  runtime.snapshot_received = true;
  const store = setUpStore({
    chat: {
      ...chat,
      current_thread_id: CHAT_ID,
      open_thread_ids: [CHAT_ID],
      threads: { [CHAT_ID]: runtime },
    },
    config: {
      host: "web",
      engineServed: true,
      lspPort: 8001,
      themeProps: {},
    },
  });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return {
    store,
    ...renderHook(() => useSkillsStatus(CHAT_ID), { wrapper }),
  };
}

describe("useSkillsStatus", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("reports the skills of a live chat session", async () => {
    server.use(
      http.get(`*/v1/chats/${CHAT_ID}/skills-status`, () =>
        HttpResponse.json(LOADED_SKILLS),
      ),
    );

    const { result } = renderSkillsStatus();

    await waitFor(() => expect(result.current.skillsAvailable).toBe(2));
    expect(result.current.skillsEnabled).toBe(true);
    expect(result.current.skillsIncluded).toEqual(["review"]);
    expect(result.current.activeSkill).toBe("review");
  });

  it("does not poll an idle chat every few seconds", async () => {
    let requests = 0;
    server.use(
      http.get(`*/v1/chats/${CHAT_ID}/skills-status`, () => {
        requests += 1;
        return HttpResponse.json(LOADED_SKILLS);
      }),
    );

    const { result } = renderSkillsStatus();

    await waitFor(() => expect(result.current.skillsAvailable).toBe(2));
    await vi.advanceTimersByTimeAsync(30_000);
    expect(requests).toBe(1);
  });

  it("refetches when a skill is activated in the chat", async () => {
    let requests = 0;
    server.use(
      http.get(`*/v1/chats/${CHAT_ID}/skills-status`, () => {
        requests += 1;
        return requests === 1
          ? new HttpResponse(null, { status: 404 })
          : HttpResponse.json(LOADED_SKILLS);
      }),
    );

    const { result, store } = renderSkillsStatus();
    await waitFor(() => expect(requests).toBe(1));
    expect(result.current.activeSkill).toBeNull();

    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "message_added",
          chat_id: CHAT_ID,
          seq: "2",
          index: 0,
          message: {
            role: "cd_instruction",
            content: "skill activated",
            message_id: "skill-1",
          },
        } as ChatEventEnvelope),
      );
    });

    await waitFor(() => expect(result.current.activeSkill).toBe("review"));
    expect(requests).toBe(2);
  });
});
