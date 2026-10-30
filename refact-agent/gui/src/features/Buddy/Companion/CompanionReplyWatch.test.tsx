import { beforeEach, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import {
  render,
  createDefaultChatState,
  waitFor,
} from "../../../utils/test-utils";
import { applyChatEvent } from "../../Chat/Thread";
import { CompanionReplyWatch } from "./CompanionReplyWatch";

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
});
it("keeps only a known busy companion subscribed until its late reply completes, then marks its source unread without navigation", async () => {
  const chat = createDefaultChatState();
  const id = chat.current_thread_id;
  chat.threads[id].streaming = true;
  chat.threads[id].thread.messages = [
    { role: "assistant", content: "Reply for A", message_id: "reply-a" },
  ];
  const unread = vi.fn();
  const { store, rerender } = render(
    <CompanionReplyWatch
      chatId={id}
      sourceId="source-a"
      reading={false}
      onUnread={unread}
    />,
    { preloadedState: { chat } },
  );
  expect(store.getState().connection.visibleChatMounts?.[id]).toBe(1);
  act(() => {
    store.dispatch(
      applyChatEvent({
        type: "runtime_updated",
        chat_id: id,
        seq: "1",
        state: "completed",
      }),
    );
  });
  await waitFor(() => expect(unread).toHaveBeenCalledWith("source-a"));
  expect(unread).toHaveBeenCalledTimes(1);
  expect(store.getState().connection.visibleChatMounts?.[id]).toBeUndefined();
  expect(store.getState().chat.current_thread_id).toBe(id);
  rerender(
    <CompanionReplyWatch
      chatId={id}
      sourceId="source-a"
      reading
      onUnread={unread}
    />,
  );
  expect(unread).toHaveBeenCalledTimes(1);
});
