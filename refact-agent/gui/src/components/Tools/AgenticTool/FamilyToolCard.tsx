import { useCallback, type ReactNode } from "react";
import { Badge, Button } from "../../ui";
import type {
  AgenticDetailRow,
  AgenticAgentRow,
  AgenticToolViewModel,
} from "./agenticToolModel";
import { shortenAgentId } from "./agenticToolModel";
import { PagedText } from "./PagedText";
import type { FamilyReport } from "./familyReport";
import {
  isSummaryBody,
  type FamilyCardState,
  type FamilyCardTexts,
} from "./familyCardState";
import styles from "./AgenticTool.module.css";

export const MAX_DETAIL_ROWS = 12;
export const MAX_AGENT_ROWS = 8;
export const MAX_AGENT_IDS = 12;
const RAW_CHARS = 2048;

function stateBadge(state: FamilyCardState, texts: FamilyCardTexts): string {
  switch (state) {
    case "pending":
      return "Awaiting result";
    case "failed":
      return "Request failed";
    case "malformed":
      return "Unreadable payload";
    case "legacy":
      return "Unrecognised response";
    case "ready":
      return texts.ready;
  }
}

export function FamilyCard({
  label,
  state,
  texts,
  model,
  content,
  children,
}: {
  label: string;
  state: FamilyCardState;
  texts: FamilyCardTexts;
  model: AgenticToolViewModel;
  content: string | null;
  children?: ReactNode;
}) {
  return (
    <section aria-label={label} className={styles.section}>
      <div className={styles.metaRow}>
        {model.historical && (
          <Badge
            tone="muted"
            variant="outline"
            data-testid="agentic-tool-historical"
          >
            Historical snapshot
          </Badge>
        )}
        <Badge tone={state === "failed" ? "danger" : "muted"}>
          {stateBadge(state, texts)}
        </Badge>
      </div>
      {model.subtitle && <p className={styles.note}>{model.subtitle}</p>}
      {state === "pending" && (
        <p role="status" data-testid="agentic-tool-pending">
          {texts.pending}
        </p>
      )}
      {state === "failed" && (
        <div
          role="alert"
          className={styles.error}
          data-testid="agentic-tool-error"
        >
          {texts.failed}
          {model.errorText ? ` ${model.errorText}` : ""}
        </div>
      )}
      {state === "malformed" && (
        <p role="alert" data-testid="agentic-tool-malformed">
          {texts.malformed}
        </p>
      )}
      {state === "malformed" && content !== null && (
        <FamilyRawText
          summary="Unparsed response"
          body={content}
          testId="agentic-tool-raw-response"
        />
      )}
      {state === "legacy" && (
        <p className={styles.note} data-testid="agentic-tool-legacy">
          {texts.legacy}
        </p>
      )}
      {children}
      <p className={styles.note}>{texts.note}</p>
    </section>
  );
}

export function FamilyDetails({
  label,
  rows,
  total,
}: {
  label: string;
  rows: AgenticDetailRow[];
  total: number;
}) {
  return (
    <section aria-label={label} className={styles.section}>
      <h4 className={styles.sectionLabel}>{label}</h4>
      <dl className={styles.detailGrid}>
        {rows.map((row, index) => (
          <div className={styles.detailItem} key={`${row.label}-${index}`}>
            <dt className={styles.detailLabel}>{row.label}</dt>
            <dd className={styles.detailValue}>{row.value}</dd>
          </div>
        ))}
      </dl>
      {total > rows.length && (
        <p className={styles.note}>
          Showing {rows.length} of {total} entries.
        </p>
      )}
    </section>
  );
}

export function FamilyTable({
  label,
  headers,
  rows,
  total,
}: {
  label: string;
  headers: string[];
  rows: FamilyReport["rows"];
  total: number;
}) {
  return (
    <section aria-label={label} className={styles.section}>
      <table className={styles.table} data-testid="agentic-tool-table">
        <caption className={styles.sectionLabel}>{label}</caption>
        <thead>
          <tr>
            {headers.map((header, index) => (
              <th
                className={styles.tableCell}
                key={`${header}-${index}`}
                scope="col"
              >
                {header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.key}>
              {row.cells.map((cell, index) => (
                <td className={styles.tableCell} key={`${row.key}-${index}`}>
                  {cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {total > rows.length && (
        <p className={styles.note}>
          Showing {rows.length} of {total} rows.
        </p>
      )}
    </section>
  );
}

export function FamilyItems({
  label,
  items,
  total,
}: {
  label: string;
  items: string[];
  total: number;
}) {
  return (
    <section aria-label={label} className={styles.section}>
      <h4 className={styles.sectionLabel}>{label}</h4>
      <ul className={styles.agentList} data-testid="agentic-tool-items">
        {items.map((item, index) => (
          <li className={styles.agentRow} key={`${item}-${index}`}>
            {item}
          </li>
        ))}
      </ul>
      {total > items.length && (
        <p className={styles.note}>
          Showing {items.length} of {total} entries.
        </p>
      )}
    </section>
  );
}

export function FamilyAgentRows({ agents }: { agents: AgenticAgentRow[] }) {
  const shown = agents.slice(0, MAX_AGENT_ROWS);
  return (
    <section aria-label="Observed agents" className={styles.section}>
      <h4 className={styles.sectionLabel}>Observed agents</h4>
      <ul className={styles.agentList} data-testid="agentic-tool-agents">
        {shown.map((agent, index) => (
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
      {agents.length > shown.length && (
        <p className={styles.note}>
          Showing {shown.length} of {agents.length} agents.
        </p>
      )}
    </section>
  );
}

export function FamilyAgentIds({ ids }: { ids: string[] }) {
  if (ids.length === 0) return null;
  const shown = ids.slice(0, MAX_AGENT_IDS);
  return (
    <details className={styles.rawDetails}>
      <summary className={styles.rawSummary}>Agent ids ({ids.length})</summary>
      <div className={styles.idList} data-testid="agentic-tool-ids">
        {shown.map((id) => (
          <Badge key={id} tone="muted" size="xs" variant="soft" title={id}>
            {shortenAgentId(id)}
          </Badge>
        ))}
      </div>
    </details>
  );
}

export function FamilyRawText({
  summary,
  body,
  testId,
}: {
  summary: string;
  body: string;
  testId: string;
}) {
  return (
    <details className={styles.rawDetails}>
      <summary className={styles.rawSummary}>{summary}</summary>
      <pre className={styles.rawContent} data-testid={testId}>
        {body.slice(0, RAW_CHARS)}
      </pre>
    </details>
  );
}

export function FamilyRawArgs({ json }: { json: string | null }) {
  if (json === null) return null;
  return (
    <FamilyRawText
      summary="Raw arguments"
      body={json}
      testId="agentic-tool-raw-args"
    />
  );
}

export function FamilyChildLink({
  childChatId,
  onOpen,
}: {
  childChatId: string;
  onOpen?: (childChatId: string) => void;
}) {
  const handleOpen = useCallback(() => {
    onOpen?.(childChatId);
  }, [childChatId, onOpen]);
  return (
    <div className={styles.actions}>
      {onOpen ? (
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
          Child trajectory {shortenAgentId(childChatId)}
        </span>
      )}
    </div>
  );
}

export function FamilyBody({
  model,
  report,
  state,
  listLabel,
  factsLabel,
}: {
  model: AgenticToolViewModel;
  report: FamilyReport;
  state: FamilyCardState;
  listLabel: string;
  factsLabel: string;
}) {
  if (state === "pending" || state === "failed" || state === "malformed")
    return null;
  const details = model.details.slice(0, MAX_DETAIL_ROWS);
  return (
    <>
      {report.title !== null && (
        <h4 className={styles.sectionLabel} data-testid="agentic-tool-title">
          {report.title}
        </h4>
      )}
      {details.length > 0 && (
        <FamilyDetails
          label={factsLabel}
          rows={details}
          total={model.details.length}
        />
      )}
      {report.fields.length > 0 && (
        <FamilyDetails
          label="Response fields"
          rows={report.fields}
          total={report.fieldTotal}
        />
      )}
      {report.rows.length > 0 && (
        <FamilyTable
          label={listLabel}
          headers={report.headers}
          rows={report.rows}
          total={report.rowTotal}
        />
      )}
      {report.items.length > 0 && (
        <FamilyItems
          label={listLabel}
          items={report.items}
          total={report.itemTotal}
        />
      )}
      <FamilyResponseText body={model.body} />
    </>
  );
}

export function FamilyResponseText({ body }: { body: string | null }) {
  if (body === null) return null;
  if (isSummaryBody(body))
    return (
      <p className={styles.detailValue} data-testid="agentic-tool-summary">
        {body}
      </p>
    );
  return <PagedText label="Response" body={body} testId="agentic-tool-body" />;
}
