import React from "react";
import { renderHook, waitFor } from "@testing-library/react";
import { Provider } from "react-redux";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import { STUB_CAPS_RESPONSE } from "../__fixtures__/caps";
import { setUpStore } from "../app/store";
import { ChatThreadProvider } from "../features/Chat/Thread";
import { setBackendStatus } from "../features/Connection";
import { useAttachedImages } from "./useAttachedImages";
import { createDefaultChatState } from "../utils/test-utils";
import { server } from "../utils/mockServer";

describe("useAttachedImages", () => {
  it("keeps the native data URL for a large PNG without canvas re-encoding", async () => {
    server.use(
      http.get("*/v1/ping", () => HttpResponse.text("pong")),
      http.get("*/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
      http.get("*/v1/chat-modes", () =>
        HttpResponse.json({ modes: [], errors: [] }),
      ),
    );
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    chat.threads[chatId].thread.model = "openai/gpt-4o";
    const store = setUpStore({
      chat,
      config: { host: "web", themeProps: {}, lspPort: 8001 },
    });
    store.dispatch(setBackendStatus({ status: "online" }));

    const nativeDataUrl = "data:image/png;base64,bmF0aXZlLTQwMDB4MjI1MQ==";
    const readAsDataURL = vi
      .spyOn(FileReader.prototype, "readAsDataURL")
      .mockImplementation(function (this: FileReader) {
        Object.defineProperty(this, "result", {
          configurable: true,
          value: nativeDataUrl,
        });
        this.dispatchEvent(new ProgressEvent("load"));
      });
    const canvasSpy = vi.spyOn(document, "createElement");
    const wrapper = ({ children }: React.PropsWithChildren) => (
      <Provider store={store}>
        <ChatThreadProvider chatId={chatId}>{children}</ChatThreadProvider>
      </Provider>
    );
    const { result } = renderHook(() => useAttachedImages(), { wrapper });

    result.current.processAndInsertImages([
      new File(["native-png-bytes"], "4000x2251.png", {
        type: "image/png",
      }),
    ]);

    await waitFor(() => {
      expect(store.getState().chat.threads[chatId]?.attached_images).toEqual([
        {
          name: "4000x2251.png",
          content: nativeDataUrl,
          type: "image/png",
        },
      ]);
    });
    expect(readAsDataURL).toHaveBeenCalledOnce();
    expect(canvasSpy).not.toHaveBeenCalledWith("canvas");
  });
});
