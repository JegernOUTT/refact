import { useCallback, useEffect, useRef, useState } from "react";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { useInternalNavigation } from "../../../hooks/useInternalNavigation";
import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import {
  ensureCompanion,
  type AttentionRow,
} from "../../../services/refact/buddyCompanion";
import { getProjectStorageNamespace } from "../../../utils/chatUiPersistence";
import { selectConfig, type Config } from "../../Config/configSlice";
import {
  createChatWithId,
  selectCurrentThreadId,
  selectIsBuddyChat,
} from "../../Chat/Thread";
import { selectVisibleChatMountIds } from "../../Connection";
import { selectFocusedWorkspaceChatId } from "../../Workspace/workspaceSlice";
import {
  focusBuddyRailButton,
  setBuddyOpen,
  updateBuddyCompanion,
  useBuddyCompanionState,
} from "./buddyCompanionStore";
import { BuddyPopover, type CompanionStatus } from "./BuddyPopover";
import { CompanionReplyWatch } from "./CompanionReplyWatch";
import { popoverStorageKey } from "./popoverGeometry";
import { useBuddyAttention } from "./useBuddyAttention";

const SOURCE_PAGES = new Set(["chat", "task workspace", "task agent"]);
const MAX_WATCHED_COMPANIONS = 8;

export function useBuddySource(): string | null {
  const workspaceFocused = useAppSelector(selectFocusedWorkspaceChatId);
  const current = useAppSelector(selectCurrentThreadId);
  const page = useAppSelector((state) => state.pages.at(-1)?.name);
  const focused =
    page && SOURCE_PAGES.has(page) ? workspaceFocused ?? current : null;
  const visible = useAppSelector((state) =>
    focused ? selectVisibleChatMountIds(state).includes(focused) : false,
  );
  const buddy = useAppSelector((state) =>
    focused ? selectIsBuddyChat(state, focused) : false,
  );
  return focused && visible && !buddy ? focused : null;
}

export function buddyNamespace(config: Config) {
  return `${getEngineEndpointIdentity(config)}:${config.host}:${
    getProjectStorageNamespace() ?? "unscoped"
  }`;
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

export function BuddyCompanion() {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const { open } = useBuddyCompanionState();
  const sourceId = useBuddySource();
  const namespace = buddyNamespace(config);
  const authorization = useRef({
    namespace,
    apiKey: config.apiKey,
    generation: 0,
  });
  if (
    authorization.current.namespace !== namespace ||
    authorization.current.apiKey !== config.apiKey
  ) {
    authorization.current = {
      namespace,
      apiKey: config.apiKey,
      generation: authorization.current.generation + 1,
    };
  }
  const session = `${namespace}:${authorization.current.generation}`;
  const attention = useBuddyAttention(config, session);
  const navigation = useInternalNavigation({ scope: namespace });
  const [companions, setCompanions] = useState<
    Partial<Record<string, CompanionStatus>>
  >({});
  const [attempt, setAttempt] = useState(0);
  const cacheKey = sourceId ? `${session}:${sourceId}` : null;
  const companion: CompanionStatus = (cacheKey && companions[cacheKey]) || {
    kind: "idle",
  };

  useEffect(() => {
    setCompanions({});
    setBuddyOpen(false);
    updateBuddyCompanion({ unread: false });
  }, [session]);

  useEffect(() => {
    updateBuddyCompanion({ attentionCount: attention.count });
  }, [attention.count]);

  useEffect(() => {
    if (!open || !sourceId || !cacheKey) return;
    const existing = companions[cacheKey];
    if (existing && existing.kind !== "idle") return;
    const captured = authorization.current;
    setCompanions((value) => ({ ...value, [cacheKey]: { kind: "loading" } }));
    ensureCompanion(config, sourceId)
      .then((meta) => {
        if (authorization.current !== captured) return;
        if (meta.source_chat_id !== sourceId)
          throw new Error("Buddy answered for a different chat.");
        dispatch(
          createChatWithId({
            id: meta.chat_id,
            title: meta.title,
            activate: false,
            openTab: false,
          }),
        );
        setCompanions((value) => ({
          ...value,
          [cacheKey]: { kind: "ready", chatId: meta.chat_id },
        }));
      })
      .catch((error: unknown) => {
        if (authorization.current !== captured) return;
        setCompanions((value) => ({
          ...value,
          [cacheKey]: { kind: "error", message: errorMessage(error) },
        }));
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- ensure once per source and retry attempt.
  }, [open, cacheKey, attempt]);

  const minimize = useCallback(() => {
    setBuddyOpen(false);
    focusBuddyRailButton();
  }, []);

  const retry = useCallback(() => {
    if (!cacheKey) return;
    setCompanions((value) => ({ ...value, [cacheKey]: { kind: "idle" } }));
    setAttempt((value) => value + 1);
  }, [cacheKey]);

  const openRow = useCallback(
    (row: AttentionRow) => {
      void navigation.navigate(
        `refact://chat/${row.parent_chat_id ?? row.chat_id}`,
      );
    },
    [navigation],
  );

  const markUnread = useCallback(() => {
    updateBuddyCompanion({ unread: true });
  }, []);

  const watched = Object.entries(companions)
    .filter(
      (entry): entry is [string, { kind: "ready"; chatId: string }] =>
        entry[0].startsWith(`${session}:`) && entry[1]?.kind === "ready",
    )
    .slice(-MAX_WATCHED_COMPANIONS);

  return (
    <>
      {watched.map(([key, value]) => (
        <CompanionReplyWatch
          key={key}
          chatId={value.chatId}
          sourceId={key.slice(session.length + 1)}
          reading={open && key === cacheKey}
          onUnread={markUnread}
        />
      ))}
      {open && (
        <BuddyPopover
          sourceId={sourceId}
          companion={companion}
          rows={attention.rows}
          storageKey={popoverStorageKey(namespace)}
          onMinimize={minimize}
          onRetry={retry}
          onOpenRow={openRow}
        />
      )}
    </>
  );
}
