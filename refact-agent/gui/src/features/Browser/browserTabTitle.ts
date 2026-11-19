import type { ProjectBrowserTab } from "../../services/refact/browser";

const MAX_URL_TITLE_LENGTH = 48;

export type BrowserTabTitleSource = {
  name?: string | null;
  title?: string | null;
  url?: string | null;
};

export const nonBlank = (
  text: string | null | undefined,
): string | undefined => (text?.trim() ? text.trim() : undefined);

const truncate = (text: string): string =>
  text.length > MAX_URL_TITLE_LENGTH
    ? `${text.slice(0, MAX_URL_TITLE_LENGTH - 1)}…`
    : text;

export function browserUrlLabel(url: string | null | undefined): string {
  const trimmed = url?.trim() ?? "";
  if (!trimmed || trimmed === "about:blank") return "";
  try {
    const parsed = new URL(trimmed);
    if (!parsed.host) return truncate(trimmed);
    const path = parsed.pathname === "/" ? "" : parsed.pathname;
    return truncate(`${parsed.host}${path}`);
  } catch {
    return truncate(trimmed);
  }
}

export function browserTabTitle(
  tab: BrowserTabTitleSource | undefined,
): string {
  return (
    nonBlank(tab?.name) ??
    nonBlank(tab?.title) ??
    nonBlank(browserUrlLabel(tab?.url)) ??
    (tab?.url?.trim() === "about:blank" ? "New tab" : "Browser tab")
  );
}

export const browserTabOwnerChatId = (
  tab: Pick<ProjectBrowserTab, "last_chat" | "opened_by_chat">,
): string | null => tab.last_chat ?? tab.opened_by_chat;

const mostRecent = (
  tabs: readonly ProjectBrowserTab[],
): ProjectBrowserTab | undefined =>
  tabs.reduce<ProjectBrowserTab | undefined>(
    (best, tab) =>
      !best ||
      (tab.last_used_ms ?? Number.NEGATIVE_INFINITY) >
        (best.last_used_ms ?? Number.NEGATIVE_INFINITY)
        ? tab
        : best,
    undefined,
  );

export const mostRecentChatTab = (
  tabs: readonly ProjectBrowserTab[],
  chatId: string,
): ProjectBrowserTab | undefined =>
  mostRecent(tabs.filter((tab) => tab.last_chat === chatId));
