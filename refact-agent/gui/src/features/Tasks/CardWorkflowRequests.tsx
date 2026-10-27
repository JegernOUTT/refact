import { useEffect, useId, useRef, useState } from "react";
import { v4 as uuidv4 } from "uuid";
import { useStore } from "react-redux";
import type { AppStore } from "../../app/store";
import {
  Button,
  Field,
  FieldSelect,
  FieldText,
  FieldTextarea,
} from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { selectConfig } from "../Config/configSlice";
import {
  selectCommandAckByThreadAndId,
  selectContextRebuildRequiredById,
  selectRuntimeById,
} from "../Chat/Thread/selectors";
import { getEngineEndpointIdentity } from "../../services/refact/apiUrl";
import { capsApi } from "../../services/refact/caps";
import { sendChatCommand } from "../../services/refact/chatCommands";
import {
  privacyApi,
  privacyDestinationForModel,
} from "../../services/refact/privacy";
import { trajectoriesApi } from "../../services/refact/trajectories";
import { toolsApi } from "../../services/refact/tools";
import {
  tasksApi,
  taskErrorMessage,
  type BoardCard,
  type TaskBoard,
} from "../../services/refact/tasks";
import { getProjectStorageNamespace } from "../../utils/chatUiPersistence";
import { selectTaskActiveChat } from "./tasksSlice";
import styles from "./CardManagement.module.css";

const useAppStore = useStore.withTypes<AppStore>();
const ACTIONS = [
  { value: "launch", label: "Launch agent", tool: "spawn_agent" },
  {
    value: "retry",
    label: "Retry using retained worktree",
    tool: "restart_agent",
  },
  {
    value: "cancel",
    label: "Cancel agent, retain worktree",
    tool: "cancel_agent",
  },
  { value: "pause", label: "Pause card", tool: "pause" },
  { value: "resume", label: "Resume card", tool: "resume" },
  { value: "verify", label: "Verify card", tool: "task_verify_card" },
  { value: "finalize", label: "Finalize card", tool: "finalize_card" },
  {
    value: "add_reviewer",
    label: "Add reviewer",
    tool: "task_board_authority",
  },
  {
    value: "remove_reviewer",
    label: "Remove reviewer",
    tool: "task_board_authority",
  },
  {
    value: "transfer_owner",
    label: "Transfer board owner",
    tool: "task_board_authority",
  },
];

function requestArgs(
  action: string,
  taskId: string,
  cardId: string,
  target: string,
) {
  if (action === "pause" || action === "resume")
    return { target: { kind: "card", task_id: taskId, card_id: cardId } };
  if (["add_reviewer", "remove_reviewer", "transfer_owner"].includes(action))
    return { action, task_id: taskId, target_chat_id: target };
  if (action === "retry")
    return { card_id: cardId, mode: "resume", force: false };
  if (action === "launch") return { card_id: cardId };
  if (action === "cancel")
    return {
      task_id: taskId,
      card_id: cardId,
      reason: "User requested cancellation; retain the worktree.",
      retain_worktree: true,
      discard_worktree: false,
    };
  return { task_id: taskId, card_id: cardId };
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function supportsArgs(
  schema: Record<string, unknown>,
  args: Record<string, unknown>,
) {
  if (schema.type !== "object") return false;
  const properties = schema.properties;
  if (!record(properties)) return false;
  if (
    Array.isArray(schema.required) &&
    schema.required.some(
      (key: unknown) => typeof key !== "string" || !(key in args),
    )
  )
    return false;
  return Object.entries(args).every(([key, value]) => {
    const field = properties[key];
    if (!record(field)) return false;
    if (key === "target") {
      return (
        Array.isArray(field.oneOf) &&
        field.oneOf.some(
          (variant: unknown) =>
            record(variant) &&
            record(variant.properties) &&
            record(variant.properties.kind) &&
            variant.properties.kind.const === "card" &&
            record(variant.properties.card_id) &&
            variant.properties.card_id.type === "string" &&
            record(variant.properties.task_id) &&
            variant.properties.task_id.type === "string" &&
            Array.isArray(variant.required) &&
            variant.required.includes("kind") &&
            variant.required.includes("card_id"),
        )
      );
    }
    return (
      field.type === typeof value &&
      (!Array.isArray(field.enum) || field.enum.includes(value))
    );
  });
}

type CardWorkflowRequestsProps = {
  taskId: string;
  card: BoardCard;
  board: TaskBoard;
  onOpenOwner: (chatId: string) => void;
};

export function CardWorkflowRequests(props: CardWorkflowRequestsProps) {
  const config = useAppSelector(selectConfig);
  const principal = useRef({ key: config.apiKey, epoch: 0 });
  if (principal.current.key !== config.apiKey) {
    principal.current = {
      key: config.apiKey,
      epoch: principal.current.epoch + 1,
    };
  }
  const identity = JSON.stringify([
    getEngineEndpointIdentity(config),
    props.taskId,
    props.card.id,
    principal.current.epoch,
  ]);
  return <CardWorkflowRequestsInstance key={identity} {...props} />;
}

function CardWorkflowRequestsInstance({
  taskId,
  card,
  board,
  onOpenOwner,
}: CardWorkflowRequestsProps) {
  const store = useAppStore();
  const fieldId = useId();
  const mounted = useRef(false);
  const inFlight = useRef(false);
  const current = useRef({ taskId, cardId: card.id });
  current.current = { taskId, cardId: card.id };
  const [action, setAction] = useState("verify");
  const [target, setTarget] = useState("");
  const [draft, setDraft] = useState(
    "Please consider this workflow request after inspecting the current board and evidence. Explain any refusal; do not bypass confirmation, review authority, dependency gates, or revision checks.",
  );
  const [baseline, setBaseline] = useState({
    owner: board.owner_chat_id,
    rev: board.rev,
  });
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [receipt, setReceipt] = useState<{
    owner: string;
    requestId: string;
    rev: number;
    phase: "sending" | "sent" | "uncertain";
  } | null>(null);
  const owner = board.owner_chat_id;
  const runtime = useAppSelector((state) =>
    owner ? selectRuntimeById(state, owner) : null,
  );
  const ack = useAppSelector((state) =>
    receipt
      ? selectCommandAckByThreadAndId(state, receipt.owner, receipt.requestId)
      : undefined,
  );
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const needsTarget = [
    "add_reviewer",
    "remove_reviewer",
    "transfer_owner",
  ].includes(action);
  const operation =
    ACTIONS.find((entry) => entry.value === action) ?? ACTIONS[5];
  const args = requestArgs(action, taskId, card.id, target.trim());
  const changed = baseline.owner !== owner || baseline.rev !== board.rev;
  const blocked = !owner
    ? "No board owner is recorded. Requests are blocked; no planner will be created or substituted."
    : !runtime?.snapshot_received
      ? "Owner planner session is not connected. Open the owner planner and wait for its snapshot."
      : !runtime.thread.model.trim()
        ? "Owner planner has no model selected. Configure its model in the owner chat."
        : runtime.prevent_send ||
            Boolean(runtime.error) ||
            runtime.confirmation.pause ||
            runtime.is_compressing
          ? "Owner planner cannot accept this request while blocked, errored, compressing, or awaiting confirmation."
          : null;

  const submit = async () => {
    if (
      inFlight.current ||
      blocked !== null ||
      changed ||
      receipt !== null ||
      !draft.trim() ||
      (needsTarget && !target.trim())
    )
      return;
    inFlight.current = true;
    setPending(true);
    setError(null);
    const initial = store.getState();
    const endpoint = getEngineEndpointIdentity(initial.config);
    const project = getProjectStorageNamespace();
    const sourcePage = initial.pages.at(-1);
    const sourceTab = initial.workspace.activeTabId;
    const sourceThread = initial.chat.current_thread_id;
    const sourceChat = selectTaskActiveChat(initial, taskId)?.chatId;
    const projectInfo = JSON.stringify(initial.current_project);
    let attemptedRequest: {
      owner: string;
      requestId: string;
      rev: number;
    } | null = null;
    let valid = true;
    const checkSource = () => {
      const state = store.getState();
      valid =
        valid &&
        mounted.current &&
        current.current.taskId === taskId &&
        current.current.cardId === card.id &&
        getEngineEndpointIdentity(state.config) === endpoint &&
        state.config.apiKey === initial.config.apiKey &&
        getProjectStorageNamespace() === project &&
        JSON.stringify(state.current_project) === projectInfo &&
        state.pages.at(-1) === sourcePage &&
        state.workspace.activeTabId === sourceTab &&
        state.chat.current_thread_id === sourceThread &&
        selectTaskActiveChat(state, taskId)?.chatId === sourceChat;
      return valid;
    };
    const unsubscribe = store.subscribe(checkSource);
    const options = { forceRefetch: true, subscribe: false };
    try {
      const task = await store
        .dispatch(tasksApi.endpoints.getTask.initiate(taskId, options))
        .unwrap();
      if (!checkSource()) return;
      if (task.id !== taskId)
        throw new Error("Task response does not match this request.");
      const latest = await store
        .dispatch(tasksApi.endpoints.getBoard.initiate(taskId, options))
        .unwrap();
      if (!checkSource()) return;
      if (!Number.isSafeInteger(latest.rev) || latest.rev < 0)
        throw new Error(
          "Current board revision is unavailable; request blocked.",
        );
      if (!latest.owner_chat_id)
        throw new Error("No board owner is recorded; request blocked.");
      if (
        latest.rev !== baseline.rev ||
        latest.owner_chat_id !== baseline.owner
      )
        throw new Error(
          "Board owner or revision changed. Review the refreshed request context before submitting again.",
        );
      if (!latest.cards.some((entry) => entry.id === card.id))
        throw new Error("Card is no longer on this board.");
      const planners = await store
        .dispatch(
          tasksApi.endpoints.listTaskTrajectories.initiate(
            { taskId, role: "planner" },
            options,
          ),
        )
        .unwrap();
      if (!checkSource()) return;
      if (!planners.some((entry) => entry.id === latest.owner_chat_id))
        throw new Error(
          "Board owner is not a visible planner for this task; request blocked.",
        );
      if (needsTarget && !planners.some((entry) => entry.id === target.trim()))
        throw new Error(
          "Target must be a currently visible authenticated planner on this task.",
        );
      const destination = await store
        .dispatch(
          trajectoriesApi.endpoints.getTrajectory.initiate(
            latest.owner_chat_id,
            options,
          ),
        )
        .unwrap();
      if (!checkSource()) return;
      if (
        destination.id !== latest.owner_chat_id ||
        typeof destination.model !== "string" ||
        !destination.model.trim()
      )
        throw new Error("Owner session or model is unavailable.");
      const caps = await store
        .dispatch(capsApi.endpoints.getCaps.initiate(undefined, options))
        .unwrap();
      if (!checkSource()) return;
      const chatModels: Partial<
        Record<string, (typeof caps.chat_models)[string]>
      > = caps.chat_models;
      if (chatModels[destination.model]?.enabled !== true)
        throw new Error(
          "Owner model is unavailable or not enabled; request blocked.",
        );
      const groups = await store
        .dispatch(toolsApi.endpoints.getToolGroups.initiate(undefined, options))
        .unwrap();
      if (!checkSource()) return;
      const tool = groups
        .flatMap((group) => group.tools)
        .find(
          (entry) =>
            entry.enabled &&
            entry.spec.name === operation.tool &&
            entry.spec.source.source_type === "builtin",
        );
      if (
        !tool ||
        !record(tool.spec.input_schema) ||
        !supportsArgs(tool.spec.input_schema, args)
      )
        throw new Error(
          "Current tool contract is unavailable or incompatible; request blocked.",
        );
      const inspection = await store
        .dispatch(
          privacyApi.endpoints.inspectPrivacy.initiate(
            {
              chat_id: destination.id,
              destination: privacyDestinationForModel(destination.model),
              records: [],
            },
            options,
          ),
        )
        .unwrap();
      if (!checkSource()) return;
      if (
        inspection.chat_id !== destination.id ||
        inspection.destination.id !==
          privacyDestinationForModel(destination.model).id ||
        (inspection.sendable as unknown) !== true
      )
        throw new Error(
          inspection.refusal ??
            "Owner model permission is unavailable; request blocked.",
        );
      const finalBoard = await store
        .dispatch(tasksApi.endpoints.getBoard.initiate(taskId, options))
        .unwrap();
      if (!checkSource()) return;
      if (
        finalBoard.rev !== latest.rev ||
        finalBoard.owner_chat_id !== latest.owner_chat_id ||
        !finalBoard.cards.some((entry) => entry.id === card.id)
      )
        throw new Error(
          "Board changed during request preparation. Review current context and submit separately.",
        );
      const ownerRuntime = selectRuntimeById(store.getState(), destination.id);
      if (
        !ownerRuntime?.snapshot_received ||
        ownerRuntime.thread.model !== destination.model ||
        ownerRuntime.prevent_send ||
        Boolean(ownerRuntime.error) ||
        ownerRuntime.confirmation.pause ||
        Boolean(ownerRuntime.is_compressing) ||
        selectContextRebuildRequiredById(store.getState(), destination.id)
      )
        throw new Error(
          "Owner session changed or cannot send. Open the owner planner to resolve it; nothing was sent.",
        );
      const content = [
        "Workflow request from task workspace — not an execution receipt.",
        `Task: ${taskId}; card: ${card.id}; observed board revision: ${finalBoard.rev}; owner: ${destination.id}.`,
        `Requested current action: ${operation.tool}(${JSON.stringify(args)}).`,
        "Resolve current task/project, board revision, ownership, permissions and evidence before considering this request. Use only your authenticated bound-task tools; preserve confirmations and CAS. Do not force, discard worktrees, take over claims, self-attest, or infer verification/finalization from this message. Report blocked or pending honestly.",
        draft.trim(),
      ].join("\n\n");
      if (!checkSource()) return;
      const requestId = uuidv4();
      attemptedRequest = {
        owner: destination.id,
        requestId,
        rev: finalBoard.rev,
      };
      setReceipt({ ...attemptedRequest, phase: "sending" });
      await sendChatCommand(
        destination.id,
        initial.config,
        initial.config.apiKey ?? undefined,
        { type: "user_message", content },
        false,
        requestId,
      );
      if (mounted.current)
        setReceipt({
          ...attemptedRequest,
          phase: checkSource() ? "sent" : "uncertain",
        });
    } catch (cause) {
      if (attemptedRequest && mounted.current)
        setReceipt({ ...attemptedRequest, phase: "uncertain" });
      if (checkSource())
        setError(
          `Request blocked or not confirmed: ${taskErrorMessage(
            cause,
          )} No automatic retry. If delivery is uncertain, inspect the owner chat before resending.`,
        );
    } finally {
      unsubscribe();
      inFlight.current = false;
      if (mounted.current) setPending(false);
    }
  };

  return (
    <section
      className={styles.form}
      aria-label="Owner planner workflow requests"
    >
      <h3>Request a protected workflow action</h3>
      <div className={styles.hint}>
        Only a message to the current board owner. Sending may start a model
        response; it does not directly execute a tool or establish an outcome.
        No planner fallback or creation.
      </div>
      <div>
        Destination: {owner ?? "unknown"} · Board revision {board.rev}
      </div>
      {blocked && <div role="status">{blocked}</div>}
      {owner && (
        <Button
          size="sm"
          variant="soft"
          disabled={pending}
          onClick={() => onOpenOwner(owner)}
        >
          Open owner planner
        </Button>
      )}
      <Field label="Workflow request" htmlFor={`${fieldId}-action`}>
        <FieldSelect
          id={`${fieldId}-action`}
          value={action}
          options={ACTIONS}
          disabled={pending || Boolean(receipt)}
          onChange={setAction}
        />
      </Field>
      {needsTarget && (
        <Field
          label="Target planner chat ID"
          htmlFor={`${fieldId}-target`}
          helper="Must already be a visible planner on this task. No planner is created."
        >
          <FieldText
            id={`${fieldId}-target`}
            value={target}
            onChange={setTarget}
            disabled={pending || Boolean(receipt)}
          />
        </Field>
      )}
      <div className={styles.hint}>
        {operation.tool}({JSON.stringify(args)})
      </div>
      <Field label="Request text" htmlFor={`${fieldId}-text`}>
        <FieldTextarea
          id={`${fieldId}-text`}
          rows={4}
          value={draft}
          onChange={setDraft}
          disabled={pending || Boolean(receipt)}
        />
      </Field>
      {changed && (
        <div role="alert">
          Board context changed. Your text is kept. Review the current owner and
          revision, then use the refreshed context before a separate submission.
        </div>
      )}
      {changed && (
        <Button
          size="sm"
          variant="soft"
          disabled={pending}
          onClick={() => {
            setBaseline({ owner, rev: board.rev });
            setError(null);
          }}
        >
          Use current owner and revision after review
        </Button>
      )}
      {error && (
        <div role="alert" className={styles.error}>
          {error}
        </div>
      )}
      {receipt ? (
        <div role="status">
          {ack
            ? ack.accepted
              ? "Planner ACK accepted"
              : "Planner ACK rejected"
            : receipt.phase === "sending"
              ? "Sending owner request; delivery is not yet confirmed"
              : receipt.phase === "uncertain"
                ? "Owner request delivery is unconfirmed; inspect the owner chat before any separate action"
                : "Request submitted; awaiting planner ACK"}
          . Owner {receipt.owner}, request {receipt.requestId}, observed
          revision {receipt.rev}. Workflow outcome pending — only updated
          board/evidence establishes results.
        </div>
      ) : (
        <Button
          size="sm"
          variant="soft"
          disabled={
            Boolean(blocked) ||
            changed ||
            pending ||
            !draft.trim() ||
            (needsTarget && !target.trim())
          }
          onClick={() => void submit()}
        >
          {pending
            ? "Preparing owner request..."
            : `Request ${operation.label.toLowerCase()} from owner planner`}
        </Button>
      )}
      <Button
        size="sm"
        variant="ghost"
        disabled={pending}
        onClick={() => {
          void store.dispatch(
            tasksApi.endpoints.getBoard.initiate(taskId, {
              forceRefetch: true,
              subscribe: false,
            }),
          );
        }}
      >
        Refresh board evidence
      </Button>
      <div className={styles.hint}>
        Review and A/B winner selection are unavailable here without an
        authenticated exact-checkout / negotiated winner contract. Inspect these
        in the owner planner; no tool alias is guessed.
      </div>
    </section>
  );
}
