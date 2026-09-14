import React from "react";
import { Send } from "lucide-react";
import { Badge, Button, Icon } from "../../ui";
import type { AgenticToolViewModel } from "./agenticToolModel";
import { shortenAgentId } from "./agenticToolModel";
import { PagedText } from "./PagedText";
import styles from "./AgenticTool.module.css";

export type AgenticToolViewProps = {
  model: AgenticToolViewModel;
  onOpenChildTrajectory?: (childChatId: string) => void;
};

export const AgenticToolView: React.FC<AgenticToolViewProps> = ({
  model,
  onOpenChildTrajectory,
}) => {
  const handleOpen = React.useCallback(() => {
    if (model.childChatId) onOpenChildTrajectory?.(model.childChatId);
  }, [model.childChatId, onOpenChildTrajectory]);

  return (
    <div className={styles.root} data-testid="agentic-tool-view">
      <div className={styles.metaRow}>
        <Badge tone="muted" size="xs" variant="soft">
          {model.family}
        </Badge>
        {model.statusLabel && (
          <Badge
            tone={model.status === "failed" ? "danger" : "warning"}
            size="xs"
            variant="soft"
            data-testid="agentic-tool-status"
          >
            {model.statusLabel}
          </Badge>
        )}
        {model.historical && (
          <Badge
            tone="muted"
            size="xs"
            variant="outline"
            data-testid="agentic-tool-historical"
          >
            historical snapshot
          </Badge>
        )}
        {model.malformedArgs && (
          <Badge
            tone="warning"
            size="xs"
            variant="soft"
            data-testid="agentic-tool-malformed"
          >
            unparsed arguments
          </Badge>
        )}
      </div>

      {model.subtitle && <div className={styles.note}>{model.subtitle}</div>}

      {model.message && (
        <section className={styles.section}>
          <span className={styles.sectionLabel}>Message</span>
          <div
            className={styles.messageBlock}
            data-testid="agentic-tool-message"
          >
            <div className={styles.messageRoute}>
              <span className={styles.messageRouteValue}>
                {model.message.sender}
              </span>
              <Icon icon={Send} size="sm" tone="muted" />
              <span className={styles.messageRouteValue}>
                {model.message.recipient}
              </span>
              {model.message.expectsReply && (
                <Badge tone="accent" size="xs" variant="soft">
                  expects reply
                </Badge>
              )}
            </div>
            <div className={styles.messageText}>{model.message.text}</div>
          </div>
        </section>
      )}

      {model.agents.length > 0 && (
        <section className={styles.section}>
          <span className={styles.sectionLabel}>Agents</span>
          <ul className={styles.agentList} data-testid="agentic-tool-agents">
            {model.agents.map((agent, index) => (
              <li
                className={styles.agentRow}
                key={`${agent.agentId}-${index}`}
                title={agent.agentId}
              >
                <span className={styles.agentId}>
                  {shortenAgentId(agent.agentId)}
                </span>
                <span className={styles.agentTitle}>{agent.title}</span>
                {agent.status && (
                  <Badge tone="muted" size="xs" variant="soft">
                    {agent.status}
                  </Badge>
                )}
                {agent.detail && (
                  <span className={styles.agentDetail}>{agent.detail}</span>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {model.details.length > 0 && (
        <section className={styles.section}>
          <span className={styles.sectionLabel}>Details</span>
          <div className={styles.detailGrid} data-testid="agentic-tool-details">
            {model.details.map((detail, index) => (
              <div
                className={styles.detailItem}
                key={`${detail.label}-${index}`}
              >
                <span className={styles.detailLabel}>{detail.label}</span>
                <span className={styles.detailValue}>{detail.value}</span>
              </div>
            ))}
          </div>
        </section>
      )}

      {model.errorText && (
        <div className={styles.error} data-testid="agentic-tool-error">
          {model.errorText}
        </div>
      )}

      {model.body && (
        <PagedText
          label="Result"
          body={model.body}
          testId="agentic-tool-body"
        />
      )}

      {model.status === "pending" && !model.body && (
        <div className={styles.note} data-testid="agentic-tool-pending">
          Waiting for the agent runtime to return a result.
        </div>
      )}

      {model.agentIds.length > 0 && (
        <details className={styles.rawDetails}>
          <summary className={styles.rawSummary}>
            Agent ids ({model.agentIds.length})
          </summary>
          <div className={styles.idList} data-testid="agentic-tool-ids">
            {model.agentIds.map((id) => (
              <Badge key={id} tone="muted" size="xs" variant="soft" title={id}>
                {shortenAgentId(id)}
              </Badge>
            ))}
          </div>
        </details>
      )}

      {model.argsJson && (
        <details className={styles.rawDetails}>
          <summary className={styles.rawSummary}>Raw arguments</summary>
          <pre
            className={styles.rawContent}
            data-testid="agentic-tool-raw-args"
          >
            {model.argsJson}
          </pre>
        </details>
      )}

      {model.childChatId && (
        <div className={styles.actions}>
          {onOpenChildTrajectory ? (
            <Button
              size="sm"
              variant="soft"
              onClick={handleOpen}
              data-testid="agentic-tool-open-child"
            >
              Open child trajectory
            </Button>
          ) : (
            <span className={styles.note} data-testid="agentic-tool-child-note">
              Child trajectory {shortenAgentId(model.childChatId)}
            </span>
          )}
        </div>
      )}
    </div>
  );
};

export default AgenticToolView;
