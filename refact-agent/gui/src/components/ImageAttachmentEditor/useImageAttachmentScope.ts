import { useAppSelector } from "../../hooks/useAppSelector";
import { useCapsForToolUse } from "../../hooks/useCapsForToolUse";
import { chatActivityIdentity } from "../../services/refact/trajectories";
import { selectIsCompanionChat, useThreadId } from "../../features/Chat/Thread";

export function useImageAttachmentScope() {
  const chatId = useThreadId();
  const identity = useAppSelector(chatActivityIdentity);
  const companion = useAppSelector((state) =>
    selectIsCompanionChat(state, chatId),
  );
  const caps = useCapsForToolUse();
  const allowed = !companion && caps.isMultimodalitySupportedForCurrentModel;
  const key = JSON.stringify([chatId, identity, caps.currentModel, allowed]);
  return { chatId, key, allowed, companion };
}
