import React from "react";
import { useAppSelector } from "../../hooks";
import {
  selectChatId,
  selectRuntimeById,
} from "../../features/Chat/Thread/selectors";
import { useRefreshToolsMutation } from "../../services/refact/trajectory";
import { getEngineEndpointIdentity } from "../../services/refact/apiUrl";
import { KitButton, KitNote } from "../ComposerKit";

export const RefreshTools: React.FC<{
  chatId?: string;
  busy: boolean;
}> = ({ chatId, busy }) => {
  const currentChatId = useAppSelector(selectChatId);
  const endpoint = useAppSelector((state) =>
    getEngineEndpointIdentity(state.config),
  );
  const credential = useAppSelector((state) => state.config.apiKey);
  const targetId = chatId ?? currentChatId;
  return (
    <RefreshToolsForChat
      key={JSON.stringify([endpoint, credential, targetId])}
      chatId={targetId}
      busy={busy}
    />
  );
};

const RefreshToolsForChat: React.FC<{ chatId: string; busy: boolean }> = ({
  chatId,
  busy,
}) => {
  const runtime = useAppSelector((state) => selectRuntimeById(state, chatId));
  const [refresh, result] = useRefreshToolsMutation();
  const disabled =
    busy ||
    result.isLoading ||
    !runtime?.snapshot_received ||
    runtime.streaming ||
    runtime.waiting_for_response ||
    runtime.prevent_send ||
    runtime.confirmation.pause ||
    (runtime.session_state !== undefined &&
      !["idle", "completed", "error"].includes(runtime.session_state)) ||
    runtime.queued_items.length > 0 ||
    runtime.is_compressing;

  return (
    <>
      <KitNote>
        Reload the current permitted tools for the next turn. Keeps all messages
        and settings; does not send a message or change permissions.
      </KitNote>
      <KitButton
        disabled={disabled}
        onClick={() => {
          if (disabled) return;
          void refresh({ chatId });
        }}
      >
        {result.isLoading ? "Refreshing tools…" : "Refresh tools for next turn"}
      </KitButton>
      {result.isSuccess ? (
        <div role="status">
          <KitNote>
            Tools refreshed for the next turn. No message was sent.
          </KitNote>
        </div>
      ) : null}
      {result.isError ? (
        <div role="alert">
          <KitNote>
            Unable to refresh tools. Wait until the chat is idle and any tool
            approvals are resolved, then try again.
          </KitNote>
        </div>
      ) : null}
    </>
  );
};
