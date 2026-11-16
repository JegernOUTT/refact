import { useEffect, useId, useMemo, useRef, useState } from "react";
import { createSelector } from "@reduxjs/toolkit";
import { AlertTriangle, OctagonX, ShieldAlert, Trash2 } from "lucide-react";
import { usePythonApprovalDecision } from "../../hooks/usePythonApprovalDecision";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  selectMessagesById,
  selectPythonApprovals,
} from "../../features/Chat/Thread/selectors";
import { selectChatWorkspaceRoots } from "../ChatContent/ToolCard/pythonRoots";
import {
  isAssistantMessage,
  type DiffChunk,
  type PythonApprovalRequest,
} from "../../services/refact/types";
import { Badge, Button, Icon, Surface } from "../ui";
import {
  ArgRows,
  ClampedText,
  ScrollText,
} from "../ChatContent/ToolCard/PythonBlocks";
import { DiffBlock } from "../ChatContent/ToolCard/DiffBlock";
import { EngineItems } from "../ChatContent/ToolCard/EngineItems";
import {
  ENGINE_CLASS_LABEL,
  approvalAnnouncement,
  isEngineTool,
  engineWarning,
  isEngineRequest,
  sentenceStart,
} from "../ChatContent/ToolCard/engineChanges";
import {
  EDIT_PAIR_KEYS,
  approvalCommand,
  editPair,
  orderedArgEntries,
  parsePythonArgs,
  relativeToRoot,
  scriptArgs,
  scriptTitle,
  truncateText,
  plural,
} from "../ChatContent/ToolCard/pythonDisplay";
import { ShortText } from "../ChatContent/ToolCard/ShortText";
import { shortText, visibleText } from "../ChatContent/ToolCard/safeText";
import styles from "./PythonApprovalBar.module.css";
import confirmationStyles from "./ToolConfirmation.module.css";

const PREVIEW_LINES = 8;
const FROM_MAX_CHARS = 120;
const TOOL_LABEL_MAX_CHARS = 60;
const SCOPE_MAX_CHARS = 160;
export const RUN_PERMIT_HINT =
  "This run: every file edit this script makes, until it ends. This chat: edits by any script in this chat.";
export const PROTECTED_HINT =
  "Protected file: this change needs your approval every time.";
/** Buttons stay disabled this long after the shown request changes. */
export const APPROVAL_SWAP_GUARD_MS = 500;
/** A request that appears this soon after the previous one left counts as a swap. */
const SWAP_WINDOW_MS = 1500;

const EMPTY_ARGUMENTS = new Map<string, string>();

let pointerPressed = false;
let pointerTracked = false;
function trackPointer() {
  if (pointerTracked) return;
  pointerTracked = true;
  const release = () => {
    pointerPressed = false;
  };
  window.addEventListener(
    "pointerdown",
    () => {
      pointerPressed = true;
    },
    true,
  );
  window.addEventListener("pointerup", release, true);
  window.addEventListener("pointercancel", release, true);
  window.addEventListener("blur", release);
}

/**
 * True while a pointer press is in progress or focus sits on something
 * outside this prompt: moving focus then would take it from the user.
 */
function userBusyElsewhere(target: HTMLElement | null) {
  if (pointerPressed) return true;
  const active = document.activeElement;
  if (!active || active === document.body) return false;
  const prompt = target?.closest("section");
  return !prompt?.contains(active);
}

/** Tool call id -> arguments of the chat's script calls, rebuilt only when messages change. */
const selectScriptCallArguments = createSelector(
  [selectMessagesById],
  (messages): ReadonlyMap<string, string> => {
    const index = new Map<string, string>();
    for (const message of messages) {
      if (!isAssistantMessage(message)) continue;
      for (const call of message.tool_calls ?? [])
        if (call.id) index.set(call.id, call.function.arguments);
    }
    return index.size > 0 ? index : EMPTY_ARGUMENTS;
  },
);

/**
 * The single action surface for script nested-call approvals, above the
 * composer. Script cards show "Waiting for approval" instead of buttons.
 */
export function PythonApprovalBar({ chatId }: { chatId: string }) {
  const requests = useAppSelector((state) =>
    selectPythonApprovals(state, chatId),
  );
  const [notice, setNotice] = useState<string>();
  useEffect(trackPointer, []);
  const oldest = requests.at(0);
  const shownId = oldest?.request_id;
  const [previousId, setPreviousId] = useState(shownId);
  const [clearedAt, setClearedAt] = useState<number>();
  const [swapped, setSwapped] = useState(false);
  if (previousId !== shownId) {
    // Another request took the place of the one shown before (directly, or
    // right after the previous one resolved): guard the buttons and drop the
    // notice that belonged to the previous request.
    const now = Date.now();
    setPreviousId(shownId);
    setClearedAt(shownId === undefined ? now : undefined);
    setSwapped(
      shownId !== undefined &&
        (previousId !== undefined ||
          (clearedAt !== undefined && now - clearedAt < SWAP_WINDOW_MS)),
    );
    if (shownId !== undefined) setNotice(undefined);
  }
  return (
    <div className={styles.list}>
      <span className={styles.srOnly} aria-live="polite">
        {oldest ? approvalAnnouncement(oldest) : ""}
      </span>
      {notice && <span className={styles.notice}>{notice}</span>}
      {oldest && (
        <PythonApprovalPrompt
          key={oldest.request_id}
          chatId={chatId}
          request={oldest}
          waiting={requests.length - 1}
          guarded={swapped}
          onExpired={() =>
            setNotice("This approval request is no longer pending.")
          }
        />
      )}
    </div>
  );
}

function PythonApprovalPrompt({
  chatId,
  request,
  waiting,
  guarded,
  onExpired,
}: {
  chatId: string;
  request: PythonApprovalRequest;
  waiting: number;
  /** True when this prompt replaced another one in place. */
  guarded: boolean;
  onExpired: () => void;
}) {
  const { pending, error, expired, decide } = usePythonApprovalDecision(
    chatId,
    request.request_id,
    onExpired,
  );
  const roots = useAppSelector((state) =>
    selectChatWorkspaceRoots(state, chatId),
  );
  const scriptArguments = useAppSelector((state) =>
    selectScriptCallArguments(state, chatId).get(request.tool_call_id),
  );
  const from = useMemo(() => {
    if (scriptArguments === undefined) return undefined;
    const { code, description } = scriptArgs(scriptArguments);
    const title = scriptTitle({ description, code });
    return { ...title, text: shortText(title.text, FROM_MAX_CHARS).shown };
  }, [scriptArguments]);
  const [settling, setSettling] = useState(guarded);
  const headingRef = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    if (!guarded) return;
    if (!userBusyElsewhere(headingRef.current)) headingRef.current?.focus();
    const timer = setTimeout(() => setSettling(false), APPROVAL_SWAP_GUARD_MS);
    return () => clearTimeout(timer);
  }, [guarded]);
  const args = useMemo(
    () => parsePythonArgs(request.args_preview),
    [request.args_preview],
  );
  const command = approvalCommand(request.command, request.tool_name, args);
  const pair = useMemo(() => editPair(args), [args]);
  const entries = useMemo(
    () =>
      args
        ? orderedArgEntries(args).filter(
            ([key, value]) =>
              !(command && key === "command" && value === command) &&
              !(pair && EDIT_PAIR_KEYS.has(key)),
          )
        : [],
    [args, command, pair],
  );
  const diff = useMemo((): DiffChunk | undefined => {
    if (!pair) return undefined;
    const path =
      typeof args?.path === "string"
        ? args.path
        : typeof args?.file_path === "string"
          ? args.file_path
          : request.tool_name;
    return {
      file_name: path,
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: pair.before,
      lines_add: pair.after,
    };
  }, [args, pair, request.tool_name]);
  const rule = request.rule.trim();
  const hasRawText = !args && !command && request.args_preview.trim() !== "";
  const nothing = !command && !diff && entries.length === 0 && !hasRawText;
  const disabled = pending || settling;
  const hintId = useId();
  const engine = isEngineRequest(request) ? request.engine : undefined;
  const careful = engine !== undefined && engine.class !== "change";
  const warning = engine
    ? engineWarning(engine.class, engine.warning)
    : undefined;
  const denyRef = useRef<HTMLButtonElement>(null);
  const focusedDeny = useRef(false);
  useEffect(() => {
    if (!careful || settling || focusedDeny.current) return;
    focusedDeny.current = true;
    if (userBusyElsewhere(denyRef.current)) return;
    denyRef.current?.focus();
  }, [careful, settling]);
  // Engine tools and protected changes never get run or chat permits.
  const protectedChange = request.needs_person === true;
  const scope = engine?.scope
    ? shortText(engine.scope, SCOPE_MAX_CHARS)
    : undefined;
  const permits =
    request.editing_tool &&
    !protectedChange &&
    !engine &&
    !isEngineTool(request.tool_name);
  const headingId = useId();
  const calloutId = useId();
  const describedBy = careful ? `${headingId} ${calloutId}` : undefined;
  if (expired) return null;
  return (
    <Surface
      as="section"
      variant="surface-1"
      className={styles.prompt}
      data-engine-class={engine?.class}
      aria-label={`Script approval for ${truncateText(
        request.tool_name,
        TOOL_LABEL_MAX_CHARS,
      )}`}
      aria-busy={settling || undefined}
    >
      <div className={styles.content}>
        {engine ? (
          <>
            <h2
              className={styles.heading}
              ref={headingRef}
              tabIndex={-1}
              id={headingId}
            >
              <Icon
                icon={AlertTriangle}
                size="sm"
                tone={engine.class === "destructive" ? "danger" : "warning"}
              />
              <span className={styles.headingText}>
                Script wants to <ShortText text={engine.title} />
              </span>
              {engine.scope && (
                <Badge
                  tone="default"
                  size="sm"
                  className={styles.scope}
                  title={scope?.full}
                  aria-description={`Scope: ${scope?.full ?? ""}`}
                >
                  Scope: {scope?.shown}
                </Badge>
              )}
            </h2>
            <ShortText text={request.tool_name} className={styles.function} />
            {from && <p className={styles.hint}>In run: “{from.text}”</p>}
            {warning && (
              <div
                className={styles.callout}
                id={calloutId}
                role="alert"
                data-tone={
                  engine.class === "destructive" ? "danger" : "warning"
                }
              >
                <Icon
                  icon={engine.class === "destructive" ? Trash2 : ShieldAlert}
                  size="sm"
                  tone={engine.class === "destructive" ? "danger" : "warning"}
                />
                <p className={styles.calloutText}>
                  <strong className={styles.calloutTitle}>
                    {ENGINE_CLASS_LABEL[engine.class]}.
                  </strong>{" "}
                  {visibleText(sentenceStart(warning))}
                </p>
              </div>
            )}
            {engine.items.length > 0 ? (
              <EngineItems items={engine.items} />
            ) : (
              <p className={styles.hint}>No values listed</p>
            )}
            {(engine.items_dropped ?? 0) > 0 && (
              <p className={styles.hint}>
                {plural(
                  engine.items_dropped ?? 0,
                  "more entry",
                  "more entries",
                )}{" "}
                could not be read and{" "}
                {engine.items_dropped === 1 ? "is" : "are"} not shown
              </p>
            )}
          </>
        ) : (
          <>
            <h2 className={styles.heading} ref={headingRef} tabIndex={-1}>
              <Icon icon={AlertTriangle} size="sm" tone="warning" />
              <span>
                Script wants to run{" "}
                <code className={styles.tool}>{request.tool_name}</code>
              </span>
              {rule && rule !== "default" && (
                <Badge tone="warning" size="sm">
                  Rule: {rule}
                </Badge>
              )}
            </h2>
            {from && <p className={styles.hint}>In run: “{from.text}”</p>}
            {command && <ScrollText text={command} label="Command" />}
            {args ? (
              <ArgRows entries={entries} root={roots} lines={PREVIEW_LINES} />
            ) : (
              hasRawText && (
                <ClampedText
                  text={request.args_preview}
                  lines={PREVIEW_LINES}
                  label="Arguments"
                />
              )
            )}
            {diff && (
              <div className={styles.diff} aria-label="Proposed change">
                <DiffBlock
                  diff={diff}
                  fileName={diff.file_name}
                  displayFileName={relativeToRoot(diff.file_name, roots)}
                />
              </div>
            )}
            {nothing && <p className={styles.hint}>No arguments</p>}
          </>
        )}
      </div>
      <div className={confirmationStyles.PatchActionRow}>
        <div
          className={confirmationStyles.PrimaryActions}
          role="group"
          aria-label="Allow"
        >
          <Button
            size="sm"
            variant={careful ? "soft" : "primary"}
            disabled={disabled}
            aria-describedby={describedBy}
            onClick={() => void decide("allow_once")}
          >
            Allow once
          </Button>
          {permits && (
            <Button
              size="sm"
              variant="soft"
              disabled={disabled}
              aria-describedby={hintId}
              onClick={() => void decide("allow_for_run")}
            >
              Allow for this run
            </Button>
          )}
          {permits && (
            <Button
              size="sm"
              variant="soft"
              disabled={disabled}
              aria-describedby={hintId}
              onClick={() => void decide("allow_for_chat")}
            >
              Allow for this chat
            </Button>
          )}
        </div>
        <div
          className={confirmationStyles.PrimaryActions}
          role="group"
          aria-label="Refuse"
        >
          <Button
            ref={denyRef}
            aria-describedby={describedBy}
            size="sm"
            variant={careful ? "primary" : "soft"}
            disabled={disabled}
            onClick={() => void decide("deny")}
          >
            Deny
          </Button>
          <Button
            leftIcon={OctagonX}
            size="sm"
            variant="danger"
            disabled={disabled}
            onClick={() => void decide("stop")}
          >
            Stop script
          </Button>
        </div>
      </div>
      {permits && (
        <p className={styles.hint} id={hintId}>
          {RUN_PERMIT_HINT}
        </p>
      )}
      {protectedChange && !engine && (
        <p className={styles.hint}>{PROTECTED_HINT}</p>
      )}
      {waiting > 0 && <p className={styles.more}>+{waiting} more waiting</p>}
      {error && (
        <p role="alert" className={styles.error}>
          {error}
        </p>
      )}
    </Surface>
  );
}
