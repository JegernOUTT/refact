import React from "react";
import { renderHook, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { Provider } from "react-redux";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../app/store";
import { useSkillsStatus } from "./useSkillsStatus";
import { server } from "../utils/mockServer";

const CHAT_ID = "chat-skills";

const LOADED_SKILLS = {
  skills_available: 2,
  skills_included: ["review"],
  skills_enabled: true,
  active_skill: "review",
};

function renderSkillsStatus() {
  const store = setUpStore({
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
  return renderHook(() => useSkillsStatus(CHAT_ID), { wrapper });
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

  it("keeps polling after a 404 and recovers once the session exists", async () => {
    let requests = 0;
    server.use(
      http.get(`*/v1/chats/${CHAT_ID}/skills-status`, () => {
        requests += 1;
        if (requests === 1) {
          return new HttpResponse(null, { status: 404 });
        }
        return HttpResponse.json(LOADED_SKILLS);
      }),
    );

    const { result } = renderSkillsStatus();

    await waitFor(() => expect(requests).toBe(1));
    expect(result.current.skillsAvailable).toBe(0);
    expect(result.current.activeSkill).toBeNull();

    await vi.advanceTimersByTimeAsync(5000);

    await waitFor(() => expect(result.current.skillsAvailable).toBe(2));
    expect(result.current.activeSkill).toBe("review");
  });
});
