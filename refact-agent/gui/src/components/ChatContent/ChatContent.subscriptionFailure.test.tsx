import { describe, expect, it } from "vitest";
import { ChatContent } from "./ChatContent";
import { setUpStore } from "../../app/store";
import { createDefaultChatState, render, screen } from "../../utils/test-utils";
import { markThreadSseError } from "../../features/Chat/Thread/actions";
import { selectSubscriptionFailureById } from "../../features/Chat/Thread/selectors";

function renderChat(busy = false, waiting = false) {
  const chat = createDefaultChatState();
  const chatId = chat.current_thread_id;
  const runtime = chat.threads[chatId];
  runtime.snapshot_received = false;
  runtime.thread.messages = [];
  if (busy) runtime.streaming = true;
  if (waiting) runtime.waiting_for_response = true;
  const store = setUpStore({ chat });
  store.dispatch(
    markThreadSseError({
      id: chatId,
      error: "Chat no longer exists",
      final: true,
    }),
  );
  const view = render(
    <ChatContent
      onRetry={() => undefined}
      onStopStreaming={() => undefined}
      onRetryGeneration={() => undefined}
    />,
    { store },
  );
  return { ...view, store, chatId };
}

describe("ChatContent subscription failure", () => {
  it("shows the close reason with Retry instead of an endless loader", async () => {
    const { store, chatId, user } = renderChat();
    expect(screen.queryByTestId("chat-loading")).not.toBeInTheDocument();
    const card = screen.getByTestId("chat-subscription-failure");
    expect(card).toHaveTextContent("This chat could not be opened");
    expect(card).toHaveTextContent("Chat no longer exists");
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(store.getState().chat.sse_refresh_requested).toBe(chatId);
    expect(selectSubscriptionFailureById(store.getState(), chatId)).toBeNull();
    expect(screen.getByTestId("chat-loading")).toBeInTheDocument();
  });

  it("keeps the busy-chat state and still shows the failure without a snapshot", () => {
    const { store, chatId } = renderChat(true);
    expect(selectSubscriptionFailureById(store.getState(), chatId)).toBe(
      "Chat no longer exists",
    );
    expect(screen.getByTestId("chat-subscription-failure")).toBeInTheDocument();
    expect(screen.queryByTestId("chat-loading")).not.toBeInTheDocument();
    const runtime = store.getState().chat.threads[chatId];
    expect(runtime?.error).toBe("Chat no longer exists");
    expect(runtime?.streaming).toBe(false);
    expect(runtime?.session_state).toBe("error");
  });

  it("shows the failure for a chat created already waiting for a response", () => {
    const { store, chatId } = renderChat(false, true);
    expect(selectSubscriptionFailureById(store.getState(), chatId)).toBe(
      "Chat no longer exists",
    );
    expect(screen.getByTestId("chat-subscription-failure")).toBeInTheDocument();
    expect(screen.queryByTestId("chat-loading")).not.toBeInTheDocument();
    expect(store.getState().chat.threads[chatId]?.waiting_for_response).toBe(
      false,
    );
  });
});
