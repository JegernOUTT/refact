import React, { useCallback, useMemo, useState } from "react";
import classNames from "classnames";
import { AlertTriangle, OctagonX } from "lucide-react";
import { useAppDispatch, useAppSelector, useChatActions } from "../../hooks";
import { Markdown } from "../Markdown";
import { Link } from "../Link";
import styles from "./ToolConfirmation.module.css";
import { push } from "../../features/Pages/pagesSlice";
import {
  isAssistantMessage,
  ToolConfirmationPauseReason,
  ToolCall,
} from "../../services/refact";
import {
  selectChatId,
  selectMessages,
  setAutoApproveEditingTools,
} from "../../features/Chat";
import { PATCH_LIKE_FUNCTIONS } from "./constants";
import { Badge, Button, Icon, Surface } from "../ui";

type ToolConfirmationProps = {
  pauseReasons: ToolConfirmationPauseReason[];
};

const getConfirmationMessage = (
  toolNames: string[],
  rules: string[],
  types: string[],
  confirmationToolNames: string[],
  denialToolNames: string[],
) => {
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
      confirmationToolNames.length > 1 ? "Commands need" : "Command needs"
    } confirmation: ${confirmationToolNames.join(", ")}.\n\nFollowing ${
      denialToolNames.length > 1 ? "commands were" : "command was"
    } denied: ${denialToolNames.join(", ")}.${
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

export const ToolConfirmation: React.FC<ToolConfirmationProps> = ({
  pauseReasons,
}) => {
  const dispatch = useAppDispatch();
  const messages = useAppSelector(selectMessages);
  const chatId = useAppSelector(selectChatId);

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
  const toolNames = resolvedReasons.map((r) => r.toolName);
  const types = resolvedReasons.map((r) => r.type);
  const rules = [...new Set(resolvedReasons.map((r) => r.rule))];

  const isPatchConfirmation = resolvedReasons.every((r) =>
    PATCH_LIKE_FUNCTIONS.includes(r.toolName),
  );

  const maybeIntegrationPath = resolvedReasons.find(
    (r) => r.integr_config_path !== null,
  )?.integr_config_path;

  const allConfirmation = resolvedReasons.every(
    (r) => r.type === "confirmation",
  );
  const isCacheGuardConfirmation =
    pauseReasons.length > 0 && pauseReasons.every(isCacheGuardReason);
  const confirmationToolNames = resolvedReasons
    .filter((r) => r.type === "confirmation")
    .map((r) => r.toolName);
  const denialToolNames = resolvedReasons
    .filter((r) => r.type === "denial")
    .map((r) => r.toolName);

  const { respondToTools } = useChatActions();

  const confirmToolUsage = useCallback(() => {
    const decisions = toolCallIds.map((id) => ({
      tool_call_id: id,
      accepted: true,
    }));
    void respondToTools(decisions);
  }, [respondToTools, toolCallIds]);

  const rejectToolUsage = useCallback(() => {
    const decisions = toolCallIds.map((id) => ({
      tool_call_id: id,
      accepted: false,
    }));
    void respondToTools(decisions);
  }, [respondToTools, toolCallIds]);

  const [isSettingAutoApprove, setIsSettingAutoApprove] = useState(false);

  const handleAllowForThisChat = useCallback(async () => {
    setIsSettingAutoApprove(true);
    try {
      const { sendChatCommand } = await import(
        "../../services/refact/chatCommands"
      );
      const state = (await import("../../app/store")).store.getState();
      const apiKey = state.config.apiKey;
      if (chatId) {
        await sendChatCommand(chatId, state.config, apiKey ?? undefined, {
          type: "set_params",
          patch: { auto_approve_editing_tools: true },
        });
      }
      dispatch(setAutoApproveEditingTools({ chatId, value: true }));
      confirmToolUsage();
    } finally {
      setIsSettingAutoApprove(false);
    }
  }, [dispatch, chatId, confirmToolUsage]);

  const handleReject = useCallback(() => {
    rejectToolUsage();
  }, [rejectToolUsage]);

  const message = getConfirmationMessage(
    toolNames,
    rules,
    types,
    confirmationToolNames,
    denialToolNames,
  );

  if (isCacheGuardConfirmation) {
    return (
      <CacheGuardConfirmation
        pauseReasons={pauseReasons}
        confirmToolUsage={confirmToolUsage}
        rejectToolUsage={handleReject}
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
        confirmToolUsage={confirmToolUsage}
        isSettingAutoApprove={isSettingAutoApprove}
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
            {resolvedReasons.map((r) => (
              <div className={styles.ToolItem} key={r.tool_call_id}>
                <Badge tone={r.type === "denial" ? "danger" : "warning"}>
                  {r.toolName}
                </Badge>
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
            {maybeIntegrationPath && (
              <p className={styles.IntegrationHint}>
                You can modify the ruleset on{" "}
                <Link
                  onClick={() => {
                    dispatch(
                      push({
                        name: "integrations page",
                        integrationPath: maybeIntegrationPath,
                        wasOpenedThroughChat: true,
                      }),
                    );
                  }}
                  color="indigo"
                >
                  Configuration Page
                </Link>
              </p>
            )}
          </div>
        </div>
        <div className={styles.ActionRow}>
          <Button variant="primary" size="sm" onClick={confirmToolUsage}>
            {allConfirmation ? "Confirm" : "Continue"}
          </Button>
          {allConfirmation && (
            <Button variant="danger" size="sm" onClick={handleReject}>
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
};

const CacheGuardConfirmation: React.FC<CacheGuardConfirmationProps> = ({
  pauseReasons,
  rejectToolUsage,
  confirmToolUsage,
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

        <div className={styles.ActionRow}>
          <Button variant="primary" size="sm" onClick={confirmToolUsage}>
            Force and Continue
          </Button>
          <Button variant="danger" size="sm" onClick={rejectToolUsage}>
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
  handleAllowForThisChat: () => Promise<void>;
  rejectToolUsage: () => void;
  confirmToolUsage: () => void;
  isSettingAutoApprove?: boolean;
};

const PatchConfirmation: React.FC<PatchConfirmationProps> = ({
  pauseReasons,
  toolCallsById,
  handleAllowForThisChat,
  confirmToolUsage,
  rejectToolUsage,
  isSettingAutoApprove,
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
        <div className={styles.PatchActionRow}>
          <div className={styles.PrimaryActions}>
            <Button
              variant="primary"
              size="sm"
              onClick={() => void handleAllowForThisChat()}
              loading={isSettingAutoApprove}
            >
              {isSettingAutoApprove ? "Setting..." : "Allow for This Chat"}
            </Button>
            <Button variant="primary" size="sm" onClick={confirmToolUsage}>
              Allow Once
            </Button>
          </div>
          <Button
            leftIcon={OctagonX}
            variant="danger"
            size="sm"
            onClick={rejectToolUsage}
          >
            Stop
          </Button>
        </div>
      </div>
    </Surface>
  );
};
