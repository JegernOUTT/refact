import { fallbackCopying } from "./fallbackCopying";

export const copyChatHistoryToClipboard = async (
  chatThread: Record<string, unknown>,
): Promise<void> => {
  const jsonString = JSON.stringify(chatThread, null, 2);

  try {
    await window.navigator.clipboard.writeText(jsonString);
  } catch {
    fallbackCopying(jsonString);
  }
};
