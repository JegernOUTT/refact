import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import { afterEach, expect, it, vi } from "vitest";

import { server } from "../../../utils/mockServer";
import { render, screen } from "../../../utils/test-utils";
import { ACTIVE_CHAT_ATTENTION_MAX_AGE_MS } from "./activeChatsModel";
import { ACTIVE_CHATS_POLL_MS, useActiveChats } from "./useActiveChats";

function Counts() {
  const { entries } = useActiveChats();
  return <output aria-label="Activity count">{entries.length}</output>;
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

it("expires unchanged cached activity on the bounded visible clock and clears the timer", async () => {
  const now = 1_800_000_000_000;
  vi.spyOn(Date, "now").mockReturnValue(now);
  vi.spyOn(document, "hidden", "get").mockReturnValue(false);
  server.use(
    http.get("*/v1/chats/activity", () =>
      HttpResponse.json({
        chats: [
          {
            id: "attention",
            title: "Attention",
            updated_at_ms: now - ACTIVE_CHAT_ATTENTION_MAX_AGE_MS,
            session_state: null,
            recovery_state: "needs_action",
            schedule_count: 0,
            paused_schedule_count: 0,
          },
        ],
      }),
    ),
  );
  const interval = vi.spyOn(window, "setInterval");
  const view = render(<Counts />);
  await screen.findByText("1");
  const clear = vi.spyOn(window, "clearInterval");
  const tick = interval.mock.calls.find(
    ([, delay]) => delay === ACTIVE_CHATS_POLL_MS,
  )?.[0];
  expect(tick).toBeTypeOf("function");
  vi.mocked(Date.now).mockReturnValue(now + ACTIVE_CHATS_POLL_MS);
  act(() => {
    if (typeof tick === "function") tick();
  });
  expect(screen.getByLabelText("Activity count")).toHaveTextContent("0");
  expect(clear).toHaveBeenCalled();
  view.unmount();
});
