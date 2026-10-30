import { useCallback, useEffect, useRef, useState } from "react";
import { Button, Sheet } from "../../../components/ui";
import { useMediaQuery } from "../../../components/ui/useMediaQuery";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { selectConfig } from "../../Config/configSlice";
import {
  createChatWithId,
  selectIsBuddyChat,
  selectThreadTitleById,
  selectCurrentThreadId,
  selectRuntimeById,
} from "../../Chat/Thread";
import { selectFocusedWorkspaceChatId } from "../../Workspace/workspaceSlice";
import { selectVisibleChatMountIds } from "../../Connection";
import { useInternalNavigation } from "../../../hooks/useInternalNavigation";
import {
  getEngineEndpointIdentity,
  hasUsableEngineEndpoint,
} from "../../../services/refact/apiUrl";
import {
  ensureCompanion,
  getAttention,
  type AttentionRow,
  type AttentionSnapshot,
} from "../../../services/refact/buddyCompanion";
import { getProjectStorageNamespace } from "../../../utils/chatUiPersistence";
import { humanizeIdentifier } from "../../../utils/displayNames";
import {
  CompanionConversation,
  type ConversationMemory,
} from "./CompanionConversation";
import { AttentionCard, type AttentionMemory } from "./AttentionCard";
import { clampGeometry, type Geometry } from "./companionPolicy";
import {
  readPendingDecisions,
  persistPendingDecision,
  restoredDecisionRow,
} from "./pendingDecisions";
import { CompanionReplyWatch } from "./CompanionReplyWatch";
import styles from "./FloatingBuddy.module.css";

const emptyMemory: ConversationMemory = { draft: "" };
export function FloatingBuddy() {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const workspaceFocused = useAppSelector(selectFocusedWorkspaceChatId);
  const current = useAppSelector(selectCurrentThreadId);
  const page = useAppSelector((state) => state.pages.at(-1)?.name);
  const focused =
    page === "chat" || page === "task workspace" || page === "task agent"
      ? workspaceFocused ?? current
      : null;
  const focusedVisible = useAppSelector((state) =>
    selectVisibleChatMountIds(state).includes(focused ?? ""),
  );
  const buddy = useAppSelector((state) =>
    selectIsBuddyChat(state, focused ?? ""),
  );
  const sourceId = focused && focusedVisible && !buddy ? focused : null;
  const sourceStatus = useAppSelector((state) => {
    const runtime = selectRuntimeById(state, sourceId ?? "");
    if (!runtime?.snapshot_received) return "No current source snapshot";
    return (
      runtime.session_state ??
      (runtime.error
        ? "error"
        : runtime.confirmation.pause
          ? "paused"
          : runtime.streaming
            ? "generating"
            : runtime.waiting_for_response
              ? "working"
              : "idle")
    );
  });
  const title = useAppSelector((state) =>
    selectThreadTitleById(state, sourceId ?? ""),
  );
  const narrow = useMediaQuery("(max-width: 600px)");
  const namespace = `${getEngineEndpointIdentity(config)}:${config.host}:${
    getProjectStorageNamespace() ?? "unscoped"
  }`;
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
  const generation = authorization.current.generation;
  const sessionNamespace = `${namespace}:${generation}`;
  const storageKey = `refact:buddy:geometry:v1:${namespace}`;
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<"ask" | "attention">("ask");
  const [compactDismissed, setCompactDismissed] = useState(false);
  const [memories, setMemories] = useState<
    Partial<Record<string, ConversationMemory>>
  >({});
  const [unread, setUnread] = useState<string[]>([]);
  const [requestScope, setRequestScope] = useState<string | null>(null);
  const [requestEvidence, setRequestEvidence] = useState<AttentionRow>();
  const attentionMemories = useRef(new Map<string, AttentionMemory>());
  const [, renderAttentionMemory] = useState(0);
  const getAttentionMemory = (id: string) => {
    const key = `${sessionNamespace}:${id}`;
    let value = attentionMemories.current.get(key);
    if (!value) {
      value = { drafts: {}, comments: "", locked: false };
      attentionMemories.current.set(key, value);
    }
    return value;
  };
  const changeAttentionMemory = (
    id: string,
    patch: Partial<AttentionMemory>,
  ) => {
    Object.assign(getAttentionMemory(id), patch);
    renderAttentionMemory((value) => value + 1);
  };
  const [snapshot, setSnapshot] = useState<AttentionSnapshot>();
  const [selected, setSelected] = useState<AttentionRow>();
  const [fresh, setFresh] = useState(false);
  const [lastChecked, setLastChecked] = useState<number>();
  const [feedError, setFeedError] = useState<string>();
  const [geometry, setGeometry] = useState<Geometry>({
    x: window.innerWidth - 460,
    y: window.innerHeight - 640,
    width: 440,
    height: 600,
  });
  const launcher = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const navigation = useInternalNavigation({ scope: namespace });
  const scope = requestScope ?? sourceId;
  const memoryKey = scope ? `${sessionNamespace}:${scope}` : "";
  const memory = memories[memoryKey] ?? emptyMemory;
  const updateMemory = useCallback(
    (key: string, patch: Partial<ConversationMemory>) => {
      setMemories((previous) => ({
        ...previous,
        [key]: { ...(previous[key] ?? emptyMemory), ...patch },
      }));
    },
    [],
  );
  useEffect(() => {
    setSnapshot(undefined);
    setSelected(undefined);
    setFresh(false);
    setRequestScope(null);
    setOpen(false);
    setCompactDismissed(false);
    setUnread([]);
  }, [sessionNamespace]);
  useEffect(() => {
    if (!hasUsableEngineEndpoint(config)) return;
    const controller = new AbortController();
    const isAborted = () => controller.signal.aborted;
    let running = false;
    const refresh = async () => {
      if (running || controller.signal.aborted) return;
      running = true;
      try {
        const next = await getAttention(config, controller.signal);
        if (isAborted()) return;
        if (next.schema_version !== 1)
          throw new Error("Unsupported Buddy attention schema");
        const recovered = readPendingDecisions(namespace).filter(
          (record) => record.project === next.project_root,
        );
        for (const record of recovered) {
          const key = `${sessionNamespace}:${record.rowId}`;
          if (!attentionMemories.current.has(key))
            attentionMemories.current.set(key, {
              drafts: {},
              comments: "",
              locked: true,
              intent: record.intent,
            });
        }
        const rows = [...next.rows];
        for (const record of recovered) {
          const index = rows.findIndex((row) => row.id === record.rowId);
          if (index < 0) rows.push(restoredDecisionRow(record));
          else if (rows[index].source_version !== record.sourceVersion)
            rows[index] = restoredDecisionRow(record);
        }
        const augmented = { ...next, rows };
        setSnapshot(augmented);
        setFresh(true);
        setLastChecked(Date.now());
        setFeedError(undefined);
        setSelected((previous) => previous ?? augmented.rows[0]);
      } catch (error) {
        if (!isAborted()) {
          setFresh(false);
          setFeedError(String(error));
        }
      } finally {
        running = false;
      }
    };
    void refresh();
    const timer = setInterval(() => {
      void refresh();
    }, 4000);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [config, namespace, sessionNamespace]);
  const createForSource = async (source: string, key: string) => {
    const captured = authorization.current;
    const meta = await ensureCompanion(config, source);
    if (authorization.current !== captured)
      throw new Error(
        "Buddy connection or authorization changed; source result ignored.",
      );
    if (meta.source_chat_id !== source)
      throw new Error("Buddy source did not match");
    dispatch(
      createChatWithId({
        id: meta.chat_id,
        title: meta.title,
        activate: false,
        openTab: false,
      }),
    );
    updateMemory(key, { chatId: meta.chat_id, error: undefined });
    return meta.chat_id;
  };
  useEffect(() => {
    try {
      const raw = localStorage.getItem(storageKey);
      if (raw) {
        const value: unknown = JSON.parse(raw);
        if (
          value &&
          typeof value === "object" &&
          ["x", "y", "width", "height"].every(
            (key) =>
              key in value &&
              typeof (value as Record<string, unknown>)[key] === "number" &&
              Number.isFinite((value as Record<string, number>)[key]),
          )
        )
          setGeometry(
            clampGeometry(
              value as Geometry,
              window.innerWidth,
              window.innerHeight,
            ),
          );
      }
    } catch {
      return;
    }
  }, [storageKey]);
  const clamp = useCallback(
    (next: Geometry) =>
      clampGeometry(
        next,
        window.visualViewport?.width ?? window.innerWidth,
        window.visualViewport?.height ?? window.innerHeight,
      ),
    [],
  );
  useEffect(() => {
    const resize = () => setGeometry((value) => clamp(value));
    resize();
    window.addEventListener("resize", resize);
    window.visualViewport?.addEventListener("resize", resize);
    return () => {
      window.removeEventListener("resize", resize);
      window.visualViewport?.removeEventListener("resize", resize);
    };
  }, [clamp]);
  useEffect(() => {
    if (!panel.current || narrow || !open) return;
    const element = panel.current;
    const observer = new ResizeObserver(() => {
      const bounds = element.getBoundingClientRect();
      if (
        Math.abs(bounds.width - geometry.width) > 2 ||
        Math.abs(bounds.height - geometry.height) > 2
      )
        setGeometry((value) =>
          clamp({ ...value, width: bounds.width, height: bounds.height }),
        );
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [geometry.width, geometry.height, clamp, open, narrow]);
  useEffect(() => {
    try {
      localStorage.setItem(storageKey, JSON.stringify(geometry));
    } catch {
      return;
    }
  }, [geometry, storageKey]);
  const minimize = () => {
    setOpen(false);
    launcher.current?.focus();
  };
  const go = (id: string) => {
    void navigation.navigate(`refact://chat/${id}`);
  };
  const next = () => {
    const rows = snapshot?.rows ?? [];
    const index = rows.findIndex((row) => row.id === selected?.id);
    setSelected(rows[(index + 1) % rows.length]);
    setCompactDismissed(false);
  };
  const ask = (row: AttentionRow) => {
    setRequestScope(row.parent_chat_id ?? row.chat_id);
    setRequestEvidence(row);
    setTab("ask");
    setOpen(true);
  };
  const count = snapshot?.rows.length ?? 0;
  const readingSource = open && tab === "ask" ? scope : null;
  useEffect(() => {
    if (readingSource)
      setUnread((values) => values.filter((id) => id !== readingSource));
  }, [readingSource]);
  const knownCompanions = Object.entries(memories)
    .filter(
      ([key, value]) =>
        key.startsWith(`${sessionNamespace}:`) && !!value?.chatId,
    )
    .slice(-8);

  const composingElsewhere = () => {
    const active = document.activeElement;
    return (
      active instanceof HTMLTextAreaElement ||
      active instanceof HTMLInputElement ||
      (active instanceof HTMLElement && active.isContentEditable)
    );
  };
  const [composing, setComposing] = useState(false);
  useEffect(() => {
    const update = () => setComposing(composingElsewhere());
    document.addEventListener("focusin", update);
    document.addEventListener("focusout", update);
    return () => {
      document.removeEventListener("focusin", update);
      document.removeEventListener("focusout", update);
    };
  }, []);
  const content = (
    <>
      <header
        className={styles.header}
        onPointerDown={(event) => {
          if (narrow || (event.target as HTMLElement).closest("button")) return;
          const start = { x: event.clientX, y: event.clientY, geometry };
          event.currentTarget.setPointerCapture(event.pointerId);
          const move = (moveEvent: PointerEvent) =>
            setGeometry(
              clamp({
                ...start.geometry,
                x: start.geometry.x + moveEvent.clientX - start.x,
                y: start.geometry.y + moveEvent.clientY - start.y,
              }),
            );
          const end = () => {
            window.removeEventListener("pointermove", move);
            window.removeEventListener("pointerup", end);
          };
          window.addEventListener("pointermove", move);
          window.addEventListener("pointerup", end, { once: true });
        }}
      >
        <Sheet.Title>Buddy</Sheet.Title>
        <Button onClick={minimize}>Minimize Buddy</Button>
      </header>
      <Sheet.Description>
        {requestScope
          ? "Request-scoped conversation"
          : "Ask about the focused source chat"}
        . Buddy uses privacy-checked source evidence, not a copied transcript.
      </Sheet.Description>
      <div className={styles.body}>
        <div className={styles.actions} role="tablist" aria-label="Buddy view">
          <button
            role="tab"
            aria-selected={tab === "ask"}
            onClick={() => setTab("ask")}
          >
            Ask Buddy
          </button>
          <button
            role="tab"
            aria-selected={tab === "attention"}
            onClick={() => setTab("attention")}
          >
            Needs attention ({count})
          </button>
        </div>
        {tab === "attention" && snapshot && selected && (
          <AttentionCard
            key={`${sessionNamespace}:${selected.id}`}
            onPersist={(intent) =>
              persistPendingDecision(
                namespace,
                snapshot.project_root,
                selected,
                intent,
              )
            }
            memory={getAttentionMemory(selected.id)}
            onMemory={(patch) => changeAttentionMemory(selected.id, patch)}
            row={selected}
            snapshot={snapshot}
            fresh={fresh}
            onNavigate={go}
            onAsk={ask}
            onNext={next}
          />
        )}
        <p role="status">
          {fresh ? "Attention checked" : "Attention unavailable / stale"}
          {lastChecked
            ? ` at ${new Date(lastChecked).toLocaleTimeString()}`
            : ""}
          .{" "}
          {snapshot &&
            `${humanizeIdentifier(snapshot.coverage)}${
              snapshot.partial ? " — partial coverage" : ""
            }. ${snapshot.busy_chat_ids.length} busy chats not inspected.`}
        </p>
        {feedError && <p role="status">{feedError}</p>}
        {navigation.error && <p role="alert">{navigation.error}</p>}
        <div className={styles.actions}>
          {scope && <Button onClick={() => go(scope)}>Open source chat</Button>}
          {requestScope && (
            <Button
              onClick={() => {
                setRequestScope(null);
                setRequestEvidence(undefined);
              }}
            >
              Return to focused source
            </Button>
          )}
        </div>
        {tab === "ask" && (
          <p role="status">
            Source status: {humanizeIdentifier(sourceStatus)}. Evidence: current
            chat snapshot; attention checks are separate.
          </p>
        )}
        <strong>
          {scope
            ? requestScope
              ? `Request source: ${requestScope}`
              : title
                ? title
                : "Current source chat"
            : "No chat selected"}
        </strong>
        {!scope && (
          <p>
            Focus a source chat to ask Buddy. Buddy chats cannot have another
            companion.
          </p>
        )}
        {tab === "ask" && scope && (
          <CompanionConversation
            key={memoryKey}
            memory={memory}
            sourceId={scope}
            requestEvidence={requestScope ? requestEvidence : undefined}
            ensure={() => createForSource(scope, memoryKey)}
            onChange={(patch) => updateMemory(memoryKey, patch)}
          />
        )}
        {!narrow && (
          <details>
            <summary>Window position and size</summary>
            <div className={styles.actions}>
              {(
                [
                  [-24, 0, "Move left"],
                  [24, 0, "Move right"],
                  [0, -24, "Move up"],
                  [0, 24, "Move down"],
                ] as const
              ).map(([x, y, label]) => (
                <Button
                  key={label}
                  onClick={() =>
                    setGeometry(
                      clamp({
                        ...geometry,
                        x: geometry.x + x,
                        y: geometry.y + y,
                      }),
                    )
                  }
                >
                  {label}
                </Button>
              ))}
              <Button
                onClick={() =>
                  setGeometry(
                    clamp({
                      ...geometry,
                      width: geometry.width + 40,
                      height: geometry.height + 40,
                    }),
                  )
                }
              >
                Larger
              </Button>
              <Button
                onClick={() =>
                  setGeometry(
                    clamp({
                      ...geometry,
                      width: geometry.width - 40,
                      height: geometry.height - 40,
                    }),
                  )
                }
              >
                Smaller
              </Button>
              <Button
                onClick={() =>
                  setGeometry(
                    clamp({
                      x: window.innerWidth - 460,
                      y: window.innerHeight - 640,
                      width: 440,
                      height: 600,
                    }),
                  )
                }
              >
                Reset window
              </Button>
            </div>
          </details>
        )}
      </div>
    </>
  );
  return (
    <>
      {knownCompanions.map(
        ([key, value]) =>
          value?.chatId && (
            <CompanionReplyWatch
              key={key}
              chatId={value.chatId}
              sourceId={key.slice(sessionNamespace.length + 1)}
              reading={readingSource === key.slice(sessionNamespace.length + 1)}
              onUnread={(source) =>
                setUnread((values) =>
                  values.includes(source) ? values : [...values, source],
                )
              }
            />
          ),
      )}
      {unread.length > 0 && (
        <aside className={styles.unread} aria-label="Unread Buddy replies">
          <span>
            {unread.length} unread Buddy{" "}
            {unread.length === 1 ? "reply" : "replies"}
          </span>
          {unread.map((source) => (
            <Button
              key={source}
              onClick={() => {
                setRequestScope(source);
                setRequestEvidence(undefined);
                setTab("ask");
                setOpen(true);
              }}
            >
              Read Buddy reply for source {source}
            </Button>
          ))}
        </aside>
      )}
      {!open && count > 0 && selected && !compactDismissed && (
        <aside className={styles.compact} aria-label="Buddy attention">
          <span>
            {composing ? "New attention — your draft is unchanged. " : ""}
            {count} requests need attention. {humanizeIdentifier(selected.kind)}
            .
          </span>
          <Button
            onClick={() => {
              setTab("attention");
              setOpen(true);
            }}
          >
            Review selected request
          </Button>
          <Button onClick={() => setCompactDismissed(true)}>
            Dismiss notification
          </Button>
          {!composing && snapshot && (
            <AttentionCard
              key={`${sessionNamespace}:${selected.id}`}
              onPersist={(intent) =>
                persistPendingDecision(
                  namespace,
                  snapshot.project_root,
                  selected,
                  intent,
                )
              }
              memory={getAttentionMemory(selected.id)}
              onMemory={(patch) => changeAttentionMemory(selected.id, patch)}
              row={selected}
              snapshot={snapshot}
              fresh={fresh}
              onNavigate={go}
              onAsk={ask}
              onNext={next}
            />
          )}
        </aside>
      )}
      <Sheet modal={narrow} open={open} onOpenChange={setOpen}>
        <Sheet.Trigger asChild>
          <button
            ref={launcher}
            className={styles.launcher}
            aria-label={`Ask Buddy${
              count ? `, ${count} attention requests` : ""
            }`}
          >
            Buddy {count > 0 && <span>{count}</span>}{" "}
            {unread.length > 0 && <span>· {unread.length} unread</span>}
          </button>
        </Sheet.Trigger>
        <Sheet.Content
          ref={panel}
          className={narrow ? styles.sheet : styles.window}
          maxWidth={narrow ? "100%" : "none"}
          maxHeight="calc(100dvh - 24px)"
          scrollable={false}
          style={
            narrow
              ? undefined
              : {
                  left: geometry.x,
                  top: geometry.y,
                  right: "auto",
                  bottom: "auto",
                  width: geometry.width,
                  height: geometry.height,
                }
          }
        >
          {content}
        </Sheet.Content>
      </Sheet>
    </>
  );
}
