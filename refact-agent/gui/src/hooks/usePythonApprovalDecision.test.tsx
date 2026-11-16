vi.hoisted(() => vi.resetModules());
import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "../utils/test-utils";
import { setUpStore } from "../app/store";
import { newChatAction, applyChatEvent } from "../features/Chat/Thread/actions";
import { pythonApproval } from "../__tests__/pythonFixtures";
import {
  trackedPythonApprovalEntries,
  usePythonApprovalDecision,
} from "./usePythonApprovalDecision";
import { sendPythonApprovalDecision } from "../services/refact/chatCommands";

vi.mock("../services/refact/chatCommands", async (original) => ({
  ...(await original<typeof import("../services/refact/chatCommands")>()),
  sendPythonApprovalDecision: vi.fn().mockResolvedValue(undefined),
}));

function Probe({ chatId, requestId }: { chatId: string; requestId: string }) {
  const { pending, decide } = usePythonApprovalDecision(
    chatId,
    requestId,
    () => undefined,
  );
  return (
    <button disabled={pending} onClick={() => void decide("allow_once")}>
      {requestId}
    </button>
  );
}

describe("usePythonApprovalDecision", () => {
  it("leaves no entries behind for requests that never reached the store", async () => {
    const store = setUpStore();
    store.dispatch(newChatAction(undefined));
    const chatId = store.getState().chat.current_thread_id;
    const view = render(<Probe chatId={chatId} requestId="ghost" />, {
      store,
    });
    expect(trackedPythonApprovalEntries(store)).toBe(1);
    fireEvent.click(screen.getByRole("button", { name: "ghost" }));
    await waitFor(() =>
      expect(sendPythonApprovalDecision).toHaveBeenCalledTimes(1),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "ghost" })).toBeEnabled(),
    );
    view.unmount();
    expect(trackedPythonApprovalEntries(store)).toBe(0);
  });
  it("keeps the lock until SSE removes the request, then drops the entry", async () => {
    const store = setUpStore();
    store.dispatch(newChatAction(undefined));
    const chatId = store.getState().chat.current_thread_id;
    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "python_approval_requested",
        request: pythonApproval,
      }),
    );
    const view = render(
      <Probe chatId={chatId} requestId={pythonApproval.request_id} />,
      { store },
    );
    const button = screen.getByRole("button", {
      name: pythonApproval.request_id,
    });
    fireEvent.click(button);
    await waitFor(() => expect(button).toBeDisabled());
    view.unmount();
    expect(trackedPythonApprovalEntries(store)).toBe(1);
    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: "2",
        type: "python_approval_resolved",
        request_id: pythonApproval.request_id,
        decision: "allow_once",
      }),
    );
    expect(trackedPythonApprovalEntries(store)).toBe(0);
  });
});
