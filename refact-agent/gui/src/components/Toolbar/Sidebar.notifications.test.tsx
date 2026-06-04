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
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [];
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

    await userEvent.click(
      screen.getByLabelText(`Engine URL ${window.location.origin}`),
    );

    expect(open).toHaveBeenCalledWith(
      window.location.origin,
      "_blank",
      "noopener,noreferrer",
    );
  });

  it("prefers an mDNS browser URL over localhost", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [
      "http://192.168.1.42:8765",
      "http://workstation.local:8765",
    ];

    render(<Toolbar activeTab={{ type: "dashboard" }} />, {
      preloadedState: {
        config: {
          host: "web",
          lspPort: 8765,
          lspUrl: "http://localhost:8765/v1/ping/Refact",
          engineServed: true,
          themeProps: {},
        },
      },
    });

    expect(
      screen.getByLabelText("Engine URL http://workstation.local:8765"),
    ).toBeInTheDocument();

    await userEvent.click(
      screen.getByLabelText("Engine URL http://workstation.local:8765"),
    );

    expect(open).toHaveBeenCalledWith(
      "http://workstation.local:8765",
      "_blank",
      "noopener,noreferrer",
    );
  });

  it("uses a LAN browser URL when mDNS is not available", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [];

    render(<Toolbar activeTab={{ type: "dashboard" }} />, {
      preloadedState: {
        config: {
          host: "vscode",
          lspPort: 8765,
          lspUrl: "http://127.0.0.1:8765/v1/ping/Refact",
          browserUrl: "http://192.168.1.42:8765/",
          themeProps: {},
        },
      },
    });

    expect(
      screen.getByLabelText("Engine URL http://192.168.1.42:8765"),
    ).toBeInTheDocument();

    await userEvent.click(
      screen.getByLabelText("Engine URL http://192.168.1.42:8765"),
    );

    expect(open).toHaveBeenCalledWith(
      "http://192.168.1.42:8765",
      "_blank",
      "noopener,noreferrer",
    );
  });

  it("opens sanitized standalone URLs without stale v1 paths", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [];

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

    await userEvent.click(
      screen.getByLabelText("Engine URL https://example.com/refact"),
    );

    expect(open).toHaveBeenCalledWith(
      "https://example.com/refact",
      "_blank",
      "noopener,noreferrer",
    );
  });
});
