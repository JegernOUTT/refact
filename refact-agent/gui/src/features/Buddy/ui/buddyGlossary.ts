export const BUDDY_DEFAULT_NAME = "Buddy";

export const BUDDY_TERMS = {
  reports: "Reports",
  report: "Report",
  requests: "Requests",
  questions: "Questions",
  question: "Question",
  approvals: "Approvals",
  approval: "Approval",
  backgroundWork: "Background work",
  recentConversations: "Recent conversations",
  backgroundChats: "Background chats",
  technicalDetails: "Technical details",
} as const;

export const BUDDY_VERBS = {
  retry: "Retry",
  refresh: "Refresh",
  viewAll: "View all",
  showMore: "Show more",
  loadMore: "Load more",
  open: "Open",
  newChat: "New chat",
  dismiss: "Dismiss",
  snooze: "Snooze",
  markRead: "Mark as read",
  markUnread: "Mark as unread",
  save: "Save",
  cancel: "Cancel",
} as const;

export function buddyLoadError(thing: string): string {
  return `Couldn't load ${thing}.`;
}

export function buddyName(name: string | null | undefined): string {
  const trimmed = name?.trim();
  return trimmed ? trimmed : BUDDY_DEFAULT_NAME;
}

export function buddyPossessive(name: string | null | undefined): string {
  const resolved = buddyName(name);
  return /s$/i.test(resolved) ? `${resolved}'` : `${resolved}'s`;
}

export function buddyTalkTo(name: string | null | undefined): string {
  return `Talk to ${buddyName(name)}`;
}
