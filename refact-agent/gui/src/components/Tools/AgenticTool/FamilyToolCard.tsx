import type { ReactNode } from "react";
import type {
  AgenticDetailRow,
  AgenticAgentRow,
  AgenticToolViewModel,
} from "./agenticToolModel";
import { shortenAgentId } from "./agenticToolModel";
import type { FamilyReport } from "./familyReport";
import type { FamilyCardState } from "./familyCardState";
import styles from "./AgenticTool.module.css";

export const MAX_DETAIL_ROWS = 12;
export const MAX_AGENT_ROWS = 8;

export function FamilyCard({
  label,
  state,
  content,
  children,
}: {
  label: string;
  state: FamilyCardState;
  content: string | null;
  children?: ReactNode;
}) {
  return (
    <section aria-label={label} className={styles.section}>
      {state === "malformed" && content !== null ? (
        <div className={styles.body} data-testid="agentic-tool-raw-response">
          {content}
        </div>
      ) : (
        children
      )}
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
    <dl aria-label={label} className={styles.detailGrid}>
      {rows.map((row, index) => (
        <div className={styles.detailItem} key={`${row.label}-${index}`}>
          <dt className={styles.detailLabel}>{row.label}</dt>
          <dd className={styles.detailValue}>{row.value}</dd>
        </div>
      ))}
      {total > rows.length && (
        <div className={styles.detailItem}>
          <dt className={styles.detailLabel}>More</dt>
          <dd className={styles.detailValue}>{total - rows.length} in Raw</dd>
        </div>
      )}
    </dl>
  );
}

export function FamilyTable({
  label,
  headers,
  rows,
}: {
  label: string;
  headers: string[];
  rows: FamilyReport["rows"];
}) {
  return (
    <table
      aria-label={label}
      className={styles.table}
      data-testid="agentic-tool-table"
    >
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
  );
}

export function FamilyItems({
  label,
  items,
}: {
  label: string;
  items: string[];
}) {
  return (
    <ul
      aria-label={label}
      className={styles.agentList}
      data-testid="agentic-tool-items"
    >
      {items.map((item, index) => (
        <li className={styles.agentRow} key={`${item}-${index}`}>
          {item}
        </li>
      ))}
    </ul>
  );
}

export function FamilyAgentRows({ agents }: { agents: AgenticAgentRow[] }) {
  const shown = agents.slice(0, MAX_AGENT_ROWS);
  return (
    <ul
      aria-label="Agents"
      className={styles.agentList}
      data-testid="agentic-tool-agents"
    >
      {shown.map((agent, index) => (
        <li
          className={styles.agentRow}
          key={`${agent.agentId}-${index}`}
          title={agent.agentId}
        >
          {agent.status && (
            <span className={styles.agentState}>{agent.status}</span>
          )}
          <span className={styles.agentTitle}>{agent.title}</span>
          <span className={styles.agentDetail}>
            {shortenAgentId(agent.agentId)}
          </span>
        </li>
      ))}
    </ul>
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
      {details.length > 0 && (
        <FamilyDetails
          label={factsLabel}
          rows={details}
          total={model.details.length}
        />
      )}
      {details.length === 0 && report.fields.length > 0 && (
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
        />
      )}
      {report.items.length > 0 && (
        <FamilyItems label={listLabel} items={report.items} />
      )}
      <FamilyResponseText body={model.body} />
    </>
  );
}

export function FamilyResponseText({ body }: { body: string | null }) {
  if (body === null) return null;
  return (
    <div className={styles.body} data-testid="agentic-tool-body">
      {body}
    </div>
  );
}
