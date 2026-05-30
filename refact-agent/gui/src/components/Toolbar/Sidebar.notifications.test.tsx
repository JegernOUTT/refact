import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import userEvent from "@testing-library/user-event";

import { render, screen } from "../../utils/test-utils";
import { Toolbar } from "./Toolbar";
import { createChatWithId, switchToThread } from "../../features/Chat/Thread";
import { processCompleted } from "../../features/Notifications";
import type { ProcessCompletedEvent } from "../../features/Notifications";

const threadId = "thread-with-notification";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function makeProcessCompletedEvent(): ProcessCompletedEvent {
  return {
    chat_id: threadId,
    seq: "3",
    type: "process_completed",
    process_id: "exec_sidebar",
    status: "failed",
    exit_code: 1,
    short_description: "Run sidebar test",
    mode: "background",
  };
}

describe("Toolbar", () => {
  it("shows the pending process completion count on the thread tab", () => {
    const { store } = render(<Toolbar activeTab={{ type: "dashboard" }} />);

    act(() => {
      store.dispatch(createChatWithId({ id: threadId, title: "Badge chat" }));
      const firstThreadId = store.getState().chat.open_thread_ids[0];
      if (!firstThreadId) throw new Error("missing initial test thread");
      store.dispatch(switchToThread({ id: firstThreadId }));
      store.dispatch(processCompleted(makeProcessCompletedEvent()));
    });

    expect(
      screen.getByLabelText("1 unread process notifications"),
    ).toHaveTextContent("1");
  });

  it("opens the current origin as an external browser link in relative mode", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);

    render(<Toolbar activeTab={{ type: "dashboard" }} />, {
      preloadedState: {
        config: {
          host: "web",
          lspPort: 8001,
          lspUrl: "http://127.0.0.1:8765/v1/ping/Refact",
          dev: true,
          themeProps: {},
        },
      },
    });

    expect(
      screen.getByLabelText(`Engine URL ${window.location.origin}`),
    ).toBeInTheDocument();

    await userEvent.click(screen.getByLabelText("Open Chat in Browser"));

    expect(open).toHaveBeenCalledWith(
      window.location.origin,
      "_blank",
      "noopener,noreferrer",
    );
  });

  it("opens sanitized standalone URLs without stale v1 paths", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);

    render(<Toolbar activeTab={{ type: "dashboard" }} />, {
      preloadedState: {
        config: {
          host: "web",
          lspPort: 8001,
          lspUrl: "https://example.com/refact/v1/ping/Refact",
          themeProps: {},
        },
      },
    });

    expect(
      screen.getByLabelText("Engine URL https://example.com/refact"),
    ).toBeInTheDocument();

    await userEvent.click(screen.getByLabelText("Open Chat in Browser"));

    expect(open).toHaveBeenCalledWith(
      "https://example.com/refact",
      "_blank",
      "noopener,noreferrer",
    );
  });
});
