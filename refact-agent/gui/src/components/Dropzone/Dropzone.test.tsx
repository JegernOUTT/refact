import { fireEvent, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import { STUB_CAPS_RESPONSE } from "../../__fixtures__/caps";
import { ChatThreadProvider } from "../../features/Chat/Thread";
import { setBackendStatus } from "../../features/Connection";
import { render } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { DropzoneProvider } from "./Dropzone";

describe("DropzoneProvider", () => {
  it("rejects an unknown binary without reading it as text", async () => {
    server.use(
      http.get("*/v1/ping", () => HttpResponse.text("pong")),
      http.get("*/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
      http.get("*/v1/chat-modes", () =>
        HttpResponse.json({ modes: [], errors: [] }),
      ),
    );
    const readAsText = vi.spyOn(FileReader.prototype, "readAsText");
    const file = new File([new Uint8Array([0, 1, 2])], "payload.bin", {
      type: "application/octet-stream",
    });
    const { container, store } = render(
      <ChatThreadProvider chatId="drop-chat">
        <DropzoneProvider>
          <div>Drop files</div>
        </DropzoneProvider>
      </ChatThreadProvider>,
      {
        preloadedState: {
          chat: {
            current_thread_id: "drop-chat",
            open_thread_ids: ["drop-chat"],
            threads: {},
            system_prompt: {},
            tool_use: "explore",
            sse_refresh_requested: null,
            stream_version: 0,
          },
          config: { host: "web", themeProps: {}, lspPort: 8001 },
        },
      },
    );
    store.dispatch({
      type: "chatThread/createChatWithId",
      payload: { id: "drop-chat", title: "Drop chat" },
    });
    store.dispatch(setBackendStatus({ status: "online" }));

    const dropzone = container.querySelector('[role="presentation"]');
    expect(dropzone).not.toBeNull();
    if (!dropzone) throw new Error("Expected dropzone root");
    fireEvent.drop(dropzone, {
      dataTransfer: {
        files: [file],
        items: [
          { kind: "file", type: file.type, getAsFile: () => file },
        ],
        types: ["Files"],
      },
    });

    await waitFor(() => {
      expect(store.getState().error.message).toBe(
        "Could not attach payload.bin: unsupported file type",
      );
    });
    expect(readAsText).not.toHaveBeenCalled();
  });
});
