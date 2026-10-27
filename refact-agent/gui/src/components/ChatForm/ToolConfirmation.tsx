import React, { useCallback, useMemo, useRef } from "react";
import classNames from "classnames";
import { AlertTriangle, OctagonX } from "lucide-react";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useChatCommandRequest } from "../../hooks/useChatCommandRequest";
import { Markdown } from "../Markdown";
import { Link } from "../Link";
import styles from "./ToolConfirmation.module.css";
import { push } from "../../features/Pages/pagesSlice";
import { selectConfig } from "../../features/Config/configSlice";
import {
  isAssistantMessage,
  ToolConfirmationPauseReason,
  ToolCall,
} from "../../services/refact";
import {
  selectMessagesById,
  selectThreadById,
  selectThreadPauseReasonsById,
  useThreadId,
} from "../../features/Chat/Thread";
import { PATCH_LIKE_FUNCTIONS } from "./constants";
import { getEngineEndpointIdentity } from "../../services/refact/apiUrl";
import { Badge, Button, Icon, Surface } from "../ui";

type ToolConfirmationProps = {
  pauseReasons: ToolConfirmationPauseReason[];
};

const getConfirmationMessage = (resolvedReasons: ResolvedPauseReason[]) => {
  const toolNames = resolvedReasons.map((r) => r.toolName);
  const types = resolvedReasons.map((r) => r.type);
  const rules = [...new Set(resolvedReasons.map((r) => r.rule))];
  const confirmationToolNames = resolvedReasons
    .filter((r) => r.type === "confirmation")
    .map((r) => r.toolName);
  const denialToolNames = resolvedReasons
    .filter((r) => r.type === "denial")
    .map((r) => r.toolName);

  const normalizedRules = rules.filter((r) => r.trim().length > 0);
  const ruleText = normalizedRules.map((r) => `\`${r}\``).join(", ");
  const ruleClause =
    normalizedRules.length > 0
      ? ` due to ${ruleText} ${normalizedRules.length > 1 ? "rules" : "rule"}`
      : "";

  if (types.every((type) => type === "confirmation")) {
    return `${
      toolNames.length > 1 ? "Commands need" : "Command needs"
    } confirmation${ruleClause}.`;
  } else if (types.every((type) => type === "denial")) {
    return `${
      toolNames.length > 1 ? "Commands were" : "Command was"
    } denied${ruleClause}.`;
  } else {
    return `${
      denialToolNames.length > 1 ? "Commands were" : "Command was"
    } denied: ${denialToolNames.join(", ")}.\n\nFollowing ${
      confirmationToolNames.length > 1 ? "commands need" : "command needs"
    } confirmation: ${confirmationToolNames.join(", ")}.${
      ruleClause ? `\n\nAll${ruleClause}.` : ""
    }`;
  }
};

type ResolvedPauseReason = {
  tool_call_id: string;
  type: string;
  rawType?: string;
  toolName: string;
  command: string;
  rule: string;
  integr_config_path: string | null;
};

function isCacheGuardReason(reason: ToolConfirmationPauseReason): boolean {
  return (
    reason.tool_name === "cache_guard" ||
    reason.tool_call_id.startsWith("cacheguard_")
  );
}

function extractCacheGuardDiff(command: string): string {
  const fenceStart = command.indexOf("```diff");
  if (fenceStart >= 0) {
    const start = fenceStart + "```diff".length;
    const fenceEnd = command.indexOf("```", start);
    if (fenceEnd > start) {
      return command.slice(start, fenceEnd).trim();
    }
  }
  return command;
}

function extractEstimatedUsd(command: string): string | null {
  const match = command.match(/`\$([0-9]+(?:\.[0-9]+)?)`\s*USD/);
  return match?.[1] ?? null;
}

function trimCommand(command: string): string {
  return command.length > 200 ? `${command.slice(0, 200)}...` : command;
}

const SHELL_TOOL_NAMES = new Set(["shell", "process_start", "shell_service"]);

function isShellConfiguration(reason: ResolvedPauseReason): boolean {
  const toolName = reason.toolName.trim().toLowerCase();
  return SHELL_TOOL_NAMES.has(toolName);
}

export const ToolConfirmation: React.FC<ToolConfirmationProps> = ({
  pauseReasons,
}) => {
  const chatId = useThreadId();
  const config = useAppSelector(selectConfig);
  const source = JSON.stringify([
    getEngineEndpointIdentity(config),
    chatId,
    pauseReasons,
  ]);
  return <ToolConfirmationInstance key={source} pauseReasons={pauseReasons} />;
};

const ToolConfirmationInstance: React.FC<ToolConfirmationProps> = ({
  pauseReasons,
}) => {
  const dispatch = useAppDispatch();
  const chatId = useThreadId();
  const messages = useAppSelector((state) => selectMessagesById(state, chatId));

  const toolCallsById = useMemo(() => {
    const map = new Map<string, ToolCall>();
    for (const m of messages) {
      if (!isAssistantMessage(m) || !m.tool_calls) continue;
      for (const tc of m.tool_calls) {
        if (tc.id) map.set(tc.id, tc);
      }
    }
    return map;
  }, [messages]);

  const resolvedReasons = useMemo((): ResolvedPauseReason[] => {
    return pauseReasons.map((r) => {
      let toolName =
        r.tool_name || toolCallsById.get(r.tool_call_id)?.function.name;
      if (!toolName) {
        const cmd = r.command.trim();
        if (cmd) {
          const firstWord = cmd.split(/\s+/)[0];
          if (firstWord && /^[a-z_]+$/.test(firstWord)) {
            toolName = firstWord;
          }
        }
      }
      return {
        tool_call_id: r.tool_call_id,
        type: r.type,
        rawType: r.raw_type,
        toolName: toolName ?? "unknown",
        command: r.command,
        rule: r.rule,
        integr_config_path: r.integr_config_path,
      };
    });
  }, [pauseReasons, toolCallsById]);

  const toolCallIds = useMemo(
    () => [...new Set(resolvedReasons.map((r) => r.tool_call_id))],
    [resolvedReasons],
  );

  const source = JSON.stringify(pauseReasons);
  const request = useChatCommandRequest(
    chatId,
    source,
    (state) =>
      JSON.stringify(selectThreadPauseReasonsById(state, chatId)) === source,
  );
  const actionLock = useRef(false);
  const [settingApplied, setSettingApplied] = React.useState(false);
  const [settingAttempted, setSettingAttempted] = React.useState(false);

  const submitDecision = useCallback(
    async (accepted: boolean, allowForChat = false) => {
      if (actionLock.current || request.blocked) return;
      actionLock.current = true;
      try {
        if (allowForChat) {
          setSettingAttempted(true);
          const applied = await request.run(
            { type: "set_params", patch: { auto_approve_editing_tools: true } },
            (state) =>
              selectThreadById(state, chatId)?.auto_approve_editing_tools ===
              true,
          );
          if (!request.isCurrent()) return;
          setSettingApplied(applied);
          if (!applied) return;
        }
        if (!request.isCurrent()) return;
        const decisions = toolCallIds.map((id) => ({
          tool_call_id: id,
          accepted,
        }));
        await request.run({ type: "tool_decisions", decisions }, (state) => {
          const current = selectThreadPauseReasonsById(state, chatId);
          return !current.some((reason) =>
            toolCallIds.includes(reason.tool_call_id),
          );
        });
      } finally {
        actionLock.current = false;
      }
    },
    [chatId, request, toolCallIds],
  );

  const confirmToolUsage = () => void submitDecision(true);
  const handleReject = () => void submitDecision(false);
  const handleAllowForThisChat = () => void submitDecision(true, true);
  const feedback = (
    <>
      {settingApplied ? (
        <p role="status" className={styles.ToolConfirmationText}>
          Chat editing approval is enabled. The current tool decision is
          separate and still awaits confirmation.
        </p>
      ) : settingAttempted && request.error ? (
        <p className={styles.ToolConfirmationText}>
          Chat editing approval was not confirmed; no tool decision was sent by
          this action.
        </p>
      ) : null}
      {request.status ? (
        <p role="status" className={styles.ToolConfirmationText}>
          {request.status}
        </p>
      ) : null}
      {request.error ? (
        <p role="alert" className={styles.ToolConfirmationText}>
          {request.error}
        </p>
      ) : null}
      {request.uncertain ? (
        <Button
          size="sm"
          variant="soft"
          disabled={request.pending}
          onClick={() => void request.checkStatus()}
        >
          Check status
        </Button>
      ) : null}
    </>
  );

  if (resolvedReasons.length === 0) return null;

  const isPatchConfirmation = resolvedReasons.every((r) =>
    PATCH_LIKE_FUNCTIONS.includes(r.toolName),
  );

  const maybeIntegrationPath = resolvedReasons.find(
    (r) => r.integr_config_path !== null,
  )?.integr_config_path;
  const linksToShellSettings = resolvedReasons.some(isShellConfiguration);

  const allConfirmation = resolvedReasons.every(
    (r) => r.type === "confirmation",
  );
  const isCacheGuardConfirmation = pauseReasons.every(isCacheGuardReason);
  const orderedReasons = [...resolvedReasons].sort((a, b) => {
    if (a.type === b.type) return 0;
    return a.type === "denial" ? -1 : 1;
  });

  const message = getConfirmationMessage(resolvedReasons);

  if (isCacheGuardConfirmation) {
    return (
      <CacheGuardConfirmation
        pauseReasons={pauseReasons}
        confirmToolUsage={confirmToolUsage}
        rejectToolUsage={handleReject}
        disabled={request.blocked}
        feedback={feedback}
      />
    );
  }

  if (isPatchConfirmation && allConfirmation) {
    return (
      <PatchConfirmation
        pauseReasons={pauseReasons}
        toolCallsById={toolCallsById}
        handleAllowForThisChat={handleAllowForThisChat}
        rejectToolUsage={handleReject}
        disabled={request.blocked}
        feedback={feedback}
        confirmToolUsage={confirmToolUsage}
      />
    );
  }

  return (
    <Surface className={styles.ToolConfirmationCard} variant="surface-1">
      <div className={styles.ToolConfirmationLayout}>
        <div className={styles.ToolConfirmationContent}>
          <div className={styles.ToolConfirmationHeading}>
            <Icon icon={AlertTriangle} size="sm" tone="warning" />
            <span>Model {allConfirmation ? "wants" : "tried"} to run:</span>
          </div>
          <div className={styles.ToolList}>
            {orderedReasons.map((r) => (
              <div
                className={styles.ToolItem}
                key={`${r.type}-${r.tool_call_id}`}
              >
                <div className={styles.ToolItemHeader}>
                  <Badge
                    tone={r.type === "denial" ? "danger" : "warning"}
                    size="md"
                  >
                    {r.toolName}
                  </Badge>
                  <Badge
                    tone={r.type === "denial" ? "danger" : "warning"}
                    size="md"
                  >
                    {r.type === "denial" ? "Denied" : "Needs confirmation"}
                  </Badge>
                </div>
                {r.command && r.command !== r.toolName && (
                  <span className={styles.CommandPreview}>
                    {trimCommand(r.command)}
                  </span>
                )}
              </div>
            ))}
          </div>
          <div className={styles.ToolConfirmationText}>
            <Markdown color="indigo">{message.concat("\n\n")}</Markdown>
            {(linksToShellSettings || maybeIntegrationPath) && (
              <p className={styles.IntegrationHint}>
                You can modify the ruleset on{" "}
                <Link
                  onClick={() => {
                    if (linksToShellSettings) {
                      dispatch(push({ name: "shell settings" }));
                    } else if (maybeIntegrationPath) {
                      dispatch(
                        push({
                          name: "integrations page",
                          integrationPath: maybeIntegrationPath,
                          wasOpenedThroughChat: true,
                        }),
                      );
                    }
                  }}
                  color="indigo"
                >
                  {linksToShellSettings
                    ? "Shell settings"
                    : "Configuration Page"}
                </Link>
              </p>
            )}
          </div>
        </div>
        {feedback}
        <div className={styles.ActionRow}>
          <Button
            variant="primary"
            size="sm"
            onClick={confirmToolUsage}
            disabled={request.blocked}
          >
            {allConfirmation ? "Confirm" : "Continue"}
          </Button>
          {allConfirmation && (
            <Button
              variant="danger"
              size="sm"
              onClick={handleReject}
              disabled={request.blocked}
            >
              Stop
            </Button>
          )}
        </div>
      </div>
    </Surface>
  );
};

type CacheGuardConfirmationProps = {
  pauseReasons: ToolConfirmationPauseReason[];
  rejectToolUsage: () => void;
  confirmToolUsage: () => void;
  disabled: boolean;
  feedback: React.ReactNode;
};

const CacheGuardConfirmation: React.FC<CacheGuardConfirmationProps> = ({
  pauseReasons,
  rejectToolUsage,
  confirmToolUsage,
  disabled,
  feedback,
}) => {
  const details = pauseReasons[0].command;
  const diff = extractCacheGuardDiff(details);
  const estimatedUsd = extractEstimatedUsd(details);

  return (
    <Surface className={styles.ToolConfirmationCard} variant="surface-1">
      <div className={styles.ToolConfirmationLayout}>
        <div className={styles.ToolConfirmationContent}>
          <div className={styles.ToolConfirmationHeading}>
            <Icon icon={AlertTriangle} size="sm" tone="warning" />
            <span>Prompt cache may be broken</span>
          </div>

          {estimatedUsd && (
            <p className={styles.ToolConfirmationText}>
              Estimated extra cost: <strong>${estimatedUsd} USD</strong>
            </p>
          )}

          <p className={styles.ToolConfirmationText}>
            Force will allow this request once and refresh cache snapshot.
          </p>

          <div className={classNames("scrollX", styles.CacheGuardScroll)}>
            <pre className={styles.CacheGuardDiff}>{diff}</pre>
          </div>
        </div>

        {feedback}
        <div className={styles.ActionRow}>
          <Button
            variant="primary"
            size="sm"
            onClick={confirmToolUsage}
            disabled={disabled}
          >
            Force and Continue
          </Button>
          <Button
            variant="danger"
            size="sm"
            onClick={rejectToolUsage}
            disabled={disabled}
          >
            Stop
          </Button>
        </div>
      </div>
    </Surface>
  );
};

type PatchConfirmationProps = {
  pauseReasons: ToolConfirmationPauseReason[];
  toolCallsById: Map<string, ToolCall>;
  handleAllowForThisChat: () => void;
  rejectToolUsage: () => void;
  confirmToolUsage: () => void;
  disabled: boolean;
  feedback: React.ReactNode;
};

const PatchConfirmation: React.FC<PatchConfirmationProps> = ({
  pauseReasons,
  toolCallsById,
  handleAllowForThisChat,
  confirmToolUsage,
  rejectToolUsage,
  disabled,
  feedback,
}) => {
  const messageForPatch = useMemo(() => {
    const filenames: string[] = [];
    for (const reason of pauseReasons) {
      const tc = toolCallsById.get(reason.tool_call_id);
      if (!tc) continue;
      try {
        const parsed = JSON.parse(tc.function.arguments) as { path?: string };
        if (parsed.path) {
          const parts = parsed.path.split(/[/\\]/);
          filenames.push(parts[parts.length - 1]);
        }
      } catch {
        continue;
      }
    }
    if (filenames.length === 0) return "Apply changes";
    const uniqueFilenames = [...new Set(filenames)];
    return `Patch ${uniqueFilenames.map((f) => `\`${f}\``).join(", ")}`;
  }, [pauseReasons, toolCallsById]);

  return (
    <Surface className={styles.ToolConfirmationCard} variant="surface-1">
      <div className={styles.ToolConfirmationLayout}>
        <div className={styles.ToolConfirmationContent}>
          <div className={styles.ToolConfirmationHeading}>
            <Icon icon={AlertTriangle} size="sm" tone="warning" />
            <span>Model wants to apply changes:</span>
          </div>
          <div className={styles.ToolConfirmationText}>
            <Markdown color="indigo">{messageForPatch.concat("\n\n")}</Markdown>
          </div>
        </div>
        {feedback}
        <div className={styles.PatchActionRow}>
          <div className={styles.PrimaryActions}>
            <Button
              variant="primary"
              size="sm"
              onClick={handleAllowForThisChat}
              disabled={disabled}
            >
              Allow for This Chat
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={confirmToolUsage}
              disabled={disabled}
            >
              Allow Once
            </Button>
          </div>
          <Button
            leftIcon={OctagonX}
            variant="danger"
            size="sm"
            onClick={rejectToolUsage}
            disabled={disabled}
          >
            Stop
          </Button>
        </div>
      </div>
    </Surface>
  );
};
