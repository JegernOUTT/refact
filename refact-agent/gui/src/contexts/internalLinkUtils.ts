import { useContext, useMemo } from "react";
import { InternalLinkContext } from "./InternalLinkContext";
import { ChatThreadContext } from "../features/Chat/Thread/ChatThreadContext";

export const useInternalLinkHandler = () => {
  const context = useContext(InternalLinkContext);
  const chatId = useContext(ChatThreadContext);
  return useMemo(() => {
    if (!context) return null;
    return {
      handleInternalLink: (url: string) =>
        context.handleInternalLink(url, chatId ? { chatId } : undefined),
    };
  }, [context, chatId]);
};

export type RefactLink =
  | { type: "chat"; id: string }
  | { type: "task"; id: string; cardId?: string }
  | { type: "worktree"; id: string; chatId?: string };

export const isRefactTargetId = (id: string): boolean =>
  /^[A-Za-z0-9_-]{1,128}$/u.test(id);

export const parseRefactLink = (url: string): RefactLink | null => {
  const match =
    /^refact:\/\/(chat|task|worktree)\/([^/?#]+)(?:\/(card|chat)\/([^/?#]+))?$/u.exec(
      url,
    );
  if (!match) return null;
  const [, type, id, suffix, targetId] = match;
  if (!isRefactTargetId(id) || (targetId && !isRefactTargetId(targetId)))
    return null;
  if (type === "chat" && !suffix) return { type, id };
  if (type === "task" && (!suffix || suffix === "card"))
    return { type, id, ...(targetId ? { cardId: targetId } : {}) };
  if (type === "worktree" && (!suffix || suffix === "chat"))
    return { type, id, ...(targetId ? { chatId: targetId } : {}) };
  return null;
};
