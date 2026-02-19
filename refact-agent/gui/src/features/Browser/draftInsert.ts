import { addInputValue } from "../../components/ChatForm/actions";

export function insertBrowserDraft(value: string): void {
  window.postMessage(
    addInputValue({
      value,
      send_immediately: false,
    }),
    "*",
  );
}

export function formatBrowserDraftBlock(
  title: string,
  content: string,
): string {
  const trimmed = content.trim();
  if (!trimmed) {
    return `[${title}] (empty)\n\n`;
  }
  return `[${title}]\n${trimmed}\n\n`;
}
