import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { Minus } from "lucide-react";
import { Button, IconButton, Text, Tooltip } from "../../../components/ui";
import { Callout } from "../../../components/Callout";
import { Portal } from "../../../components/Portal";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { humanizeIdentifier } from "../../../utils/displayNames";
import type { AttentionRow } from "../../../services/refact/buddyCompanion";
import { selectConfig } from "../../Config/configSlice";
import { Chat } from "../../Chat/Chat";
import { selectRuntimeById, selectThreadTitleById } from "../../Chat/Thread";
import {
  registerVisibleChatMount,
  unregisterVisibleChatMount,
} from "../../Connection";
import { getBuddyRailButton } from "./buddyCompanionStore";
import {
  clampGeometry,
  currentViewport,
  defaultGeometry,
  movedGeometry,
  readStoredGeometry,
  resizedGeometry,
  writeStoredGeometry,
  type Geometry,
} from "./popoverGeometry";
import styles from "./BuddyPopover.module.css";

export const NEEDS_YOU_PREVIEW = 3;

export type CompanionStatus =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ready"; chatId: string }
  | { kind: "error"; message: string };

export type BuddyPopoverProps = {
  sourceId: string | null;
  companion: CompanionStatus;
  rows: AttentionRow[];
  storageKey: string;
  onMinimize: () => void;
  onRetry: () => void;
  onOpenRow: (row: AttentionRow) => void;
};

type DragState = {
  pointerId: number;
  mode: "move" | "resize";
  startX: number;
  startY: number;
  origin: Geometry;
};

const KEYBOARD_STEP = 16;
const KEYBOARD_STEP_LARGE = 64;
const ARROW_DIRECTIONS: Partial<Record<string, readonly [number, number]>> = {
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
  ArrowUp: [0, -1],
  ArrowDown: [0, 1],
};
const ARROW_SHORTCUTS = "ArrowUp ArrowDown ArrowLeft ArrowRight";
const INVISIBLE_CHARACTERS =
  // eslint-disable-next-line no-control-regex
  /[\u0000-\u001f\u007f\u200e\u200f\u202a-\u202e\u2066-\u2069]+/g;

function visibleText(value: string) {
  return value.replace(INVISIBLE_CHARACTERS, " ").trim();
}

function initialGeometry(storageKey: string): Geometry {
  const viewport = currentViewport();
  const stored = readStoredGeometry(storageKey);
  if (stored) return clampGeometry(stored, viewport);
  const button = getBuddyRailButton();
  if (!button) return defaultGeometry(viewport, null);
  const bounds = button.getBoundingClientRect();
  const rail = button.closest("nav")?.getBoundingClientRect() ?? bounds;
  return defaultGeometry(viewport, {
    right: rail.right,
    bottom: bounds.bottom,
  });
}

function NeedsYouRow({
  row,
  onOpen,
}: {
  row: AttentionRow;
  onOpen: (row: AttentionRow) => void;
}) {
  const chatId = row.parent_chat_id ?? row.chat_id;
  const title = useAppSelector(
    (state) =>
      selectThreadTitleById(state, row.chat_id) ??
      selectThreadTitleById(state, chatId),
  );
  const kind = humanizeIdentifier(row.kind);
  const chatTitle = visibleText(title ?? "") || "Untitled chat";
  const reason = visibleText(row.reason);
  return (
    <li>
      <button
        type="button"
        className={styles.row}
        onClick={() => onOpen(row)}
        title={`${kind} · ${chatTitle}\n${reason}`}
      >
        <span className={styles.rowHead}>
          <span className={styles.rowKind}>{kind}</span>
          <span aria-hidden="true">·</span>
          <span className={styles.rowTitle}>{chatTitle}</span>
        </span>
        <span className={styles.rowReason}>{reason}</span>
      </button>
    </li>
  );
}

function NeedsYou({
  rows,
  onOpen,
}: {
  rows: AttentionRow[];
  onOpen: (row: AttentionRow) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  if (rows.length === 0) return null;
  const visible = expanded ? rows : rows.slice(0, NEEDS_YOU_PREVIEW);
  return (
    <section className={styles.needsYou} aria-label="Needs you">
      <Text as="h3" size="1" weight="medium" color="gray">
        Needs you
      </Text>
      <ul className={styles.rows}>
        {visible.map((row) => (
          <NeedsYouRow key={row.id} row={row} onOpen={onOpen} />
        ))}
      </ul>
      {rows.length > NEEDS_YOU_PREVIEW && (
        <Button
          size="sm"
          variant="plain"
          className={styles.showAll}
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
        >
          {expanded ? "Show less" : `Show all (${rows.length})`}
        </Button>
      )}
    </section>
  );
}

const CompanionChat = memo(function CompanionChat({
  chatId,
}: {
  chatId: string;
}) {
  const config = useAppSelector(selectConfig);
  const backFromChat = useCallback(() => undefined, []);
  return (
    <Chat
      host={config.host}
      tabbed={config.tabbed}
      backFromChat={backFromChat}
      chatId={chatId}
    />
  );
});

export function BuddyPopover({
  sourceId,
  companion,
  rows,
  storageKey,
  onMinimize,
  onRetry,
  onOpenRow,
}: BuddyPopoverProps) {
  const dispatch = useAppDispatch();
  const sectionRef = useRef<HTMLElement>(null);
  const drag = useRef<DragState | null>(null);
  const [geometry, setGeometry] = useState<Geometry>(() =>
    initialGeometry(storageKey),
  );
  const geometryRef = useRef(geometry);
  const intendedRef = useRef(geometry);
  const sourceTitle = useAppSelector((state) =>
    sourceId ? selectThreadTitleById(state, sourceId) : undefined,
  );
  const companionId = companion.kind === "ready" ? companion.chatId : null;
  const companionReady = useAppSelector((state) =>
    companionId
      ? selectRuntimeById(state, companionId)?.snapshot_received ?? false
      : false,
  );

  useEffect(() => {
    if (!companionId || companionReady) return;
    dispatch(registerVisibleChatMount({ chatId: companionId }));
    return () => {
      dispatch(unregisterVisibleChatMount({ chatId: companionId }));
    };
  }, [companionId, companionReady, dispatch]);

  const applyGeometry = (next: Geometry) => {
    geometryRef.current = next;
    setGeometry(next);
  };

  useEffect(() => {
    const clampToViewport = () => {
      const next = clampGeometry(intendedRef.current, currentViewport());
      geometryRef.current = next;
      setGeometry(next);
    };
    window.addEventListener("resize", clampToViewport);
    window.visualViewport?.addEventListener("resize", clampToViewport);
    return () => {
      window.removeEventListener("resize", clampToViewport);
      window.visualViewport?.removeEventListener("resize", clampToViewport);
    };
  }, []);

  useLayoutEffect(() => {
    sectionRef.current?.focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    const onDocumentKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      const active = document.activeElement;
      if (active && active !== document.body) return;
      event.preventDefault();
      onMinimize();
    };
    document.addEventListener("keydown", onDocumentKeyDown);
    return () => document.removeEventListener("keydown", onDocumentKeyDown);
  }, [onMinimize]);

  useEffect(() => {
    if (!companionId || !companionReady) return;
    const frame = window.requestAnimationFrame(() => {
      const section = sectionRef.current;
      const active = document.activeElement;
      if (
        !section ||
        (active &&
          active !== document.body &&
          active !== section &&
          !section.contains(active) &&
          active !== getBuddyRailButton())
      )
        return;
      section
        .querySelector<HTMLTextAreaElement>("[data-testid=chat-form-textarea]")
        ?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [companionId, companionReady]);

  const beginDrag = (
    event: React.PointerEvent<HTMLElement>,
    mode: DragState["mode"],
  ) => {
    if (event.button !== 0) return;
    drag.current = {
      pointerId: event.pointerId,
      mode,
      startX: event.clientX,
      startY: event.clientY,
      origin: geometryRef.current,
    };
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      drag.current = { ...drag.current };
    }
    event.preventDefault();
  };

  const moveDrag = (event: React.PointerEvent<HTMLElement>) => {
    const state = drag.current;
    if (!state || state.pointerId !== event.pointerId) return;
    const update = state.mode === "move" ? movedGeometry : resizedGeometry;
    applyGeometry(
      update(
        state.origin,
        event.clientX - state.startX,
        event.clientY - state.startY,
        currentViewport(),
      ),
    );
  };

  const endDrag = (event: React.PointerEvent<HTMLElement>) => {
    if (drag.current?.pointerId !== event.pointerId) return;
    drag.current = null;
    intendedRef.current = geometryRef.current;
    writeStoredGeometry(storageKey, geometryRef.current);
    try {
      event.currentTarget.releasePointerCapture(event.pointerId);
    } catch {
      return;
    }
  };

  const nudge = (
    event: React.KeyboardEvent<HTMLElement>,
    mode: DragState["mode"],
  ) => {
    const direction = ARROW_DIRECTIONS[event.key];
    if (!direction) return;
    event.preventDefault();
    const step = event.shiftKey ? KEYBOARD_STEP_LARGE : KEYBOARD_STEP;
    const update = mode === "move" ? movedGeometry : resizedGeometry;
    const next = update(
      geometryRef.current,
      direction[0] * step,
      direction[1] * step,
      currentViewport(),
    );
    applyGeometry(next);
    intendedRef.current = next;
    writeStoredGeometry(storageKey, next);
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key !== "Escape" || event.defaultPrevented) return;
    const target = event.target;
    if (!(target instanceof Element) || !sectionRef.current?.contains(target))
      return;
    if (target.closest('[aria-expanded="true"]')) return;
    event.preventDefault();
    onMinimize();
  };

  const headerTitle = sourceId
    ? sourceTitle?.trim()
      ? sourceTitle
      : "Untitled chat"
    : null;

  return (
    <Portal>
      <section
        ref={sectionRef}
        role="dialog"
        aria-modal="false"
        aria-label="Buddy"
        tabIndex={-1}
        className={styles.popover}
        data-buddy-popover=""
        style={{
          left: geometry.x,
          top: geometry.y,
          width: geometry.width,
          height: geometry.height,
        }}
        onKeyDown={onKeyDown}
      >
        <header
          className={styles.header}
          data-testid="buddy-popover-header"
          onPointerDown={(event) => {
            if (
              event.target instanceof Element &&
              event.target.closest("button, a, input, textarea, select")
            )
              return;
            beginDrag(event, "move");
          }}
          onPointerMove={moveDrag}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        >
          <span
            className={styles.heading}
            role="button"
            tabIndex={0}
            aria-label={
              headerTitle ? `Move Buddy · ${headerTitle}` : "Move Buddy"
            }
            aria-keyshortcuts={ARROW_SHORTCUTS}
            onKeyDown={(event) => nudge(event, "move")}
          >
            <Text as="h2" size="2" weight="bold" className={styles.title}>
              Buddy
            </Text>
            {headerTitle && (
              <span className={styles.subtitle} title={headerTitle}>
                {headerTitle}
              </span>
            )}
          </span>
          <Tooltip content="Minimize Buddy">
            <IconButton
              icon={Minus}
              size="sm"
              variant="ghost"
              aria-label="Minimize Buddy"
              onClick={onMinimize}
            />
          </Tooltip>
        </header>
        <div className={styles.body}>
          <NeedsYou rows={rows} onOpen={onOpenRow} />
          {!sourceId && (
            <Text as="p" size="2" color="gray" className={styles.hint}>
              Open a chat to talk to Buddy about it.
            </Text>
          )}
          {sourceId && companion.kind === "error" && (
            <div className={styles.status}>
              <Callout type="error" preventClose>
                Buddy is unavailable right now. {companion.message}
              </Callout>
              <Button size="sm" variant="soft" onClick={onRetry}>
                Retry
              </Button>
            </div>
          )}
          {sourceId &&
            (companion.kind === "loading" ||
              (companionId && !companionReady)) && (
              <Text
                as="p"
                size="2"
                color="gray"
                className={styles.hint}
                role="status"
              >
                Loading Buddy…
              </Text>
            )}
          {companionId && companionReady && (
            <div className={styles.chat}>
              <CompanionChat key={companionId} chatId={companionId} />
            </div>
          )}
        </div>
        <div
          className={styles.resizeHandle}
          data-testid="buddy-resize-handle"
          role="button"
          tabIndex={0}
          aria-label="Resize Buddy"
          aria-keyshortcuts={ARROW_SHORTCUTS}
          onKeyDown={(event) => nudge(event, "resize")}
          onPointerDown={(event) => beginDrag(event, "resize")}
          onPointerMove={moveDrag}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        />
      </section>
    </Portal>
  );
}
