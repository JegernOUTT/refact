import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import type { setUpStore } from "../../src/app/store";
import { setChatModel } from "../../src/features/Chat/Thread/actions";
import { ChatThreadProvider } from "../../src/features/Chat/Thread";
import { RetryForm } from "../../src/components/ChatForm/RetryForm";
import { DropzoneProvider } from "../../src/components/Dropzone";
import { Theme } from "../../src/components/Theme";
const retry = new URLSearchParams(location.search).has("retry");
history.replaceState(null, "", "?route=chat-dnd&toolbar_alignment=1&unsplit=1");
await import("./route-showcase");
const store = (
  window as unknown as {
    __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
  }
).__toolbarAlignmentStore;
store.dispatch(
  setChatModel({
    chatId: "showcase-chat",
    model: "openai_codex_personal/gpt-5.5",
  }),
);
if (retry) {
  createRoot(document.getElementById("refact-chat")!).render(
    <Provider store={store}>
      <Theme>
        <ChatThreadProvider chatId="showcase-chat">
          <DropzoneProvider>
            <RetryForm
              value="before OLD after"
              onClose={() => undefined}
              onSubmit={(value) =>
                Object.assign(window, { __retrySubmitted: value })
              }
            />
          </DropzoneProvider>
        </ChatThreadProvider>
      </Theme>
    </Provider>,
  );
}
