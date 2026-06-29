import React from "react";
import {
  Bot,
  CheckCircle2,
  Clock4,
  GitBranch,
  Layers,
  ListChecks,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";
import { Surface } from "../../../components/ui";
import { useGetStatsSummaryQuery } from "../../../services/refact/stats";
import { Spinner } from "../../../components/Spinner";
import { ErrorCallout } from "../../../components/Callout";
import { StatCard } from "../components/StatCard";
import { StatSection } from "../components/StatSection";
import {
  formatTokenCount,
  formatCostDisplay,
  formatCompact,
  formatDuration,
  formatNumber,
  formatRatioPercent,
  formatRelativeTime,
  shortId,
} from "../utils/formatters";
import { dateRangeToApiArgs } from "../utils/dateRange";
import type { CountItem, DateRange, HourStats } from "../types";
import styles from "./TasksTab.module.css";

type Props = { dateRange: DateRange };

function titleCase(key: string): string {
  return key
    .split("_")
    .map((part) => (part ? part[0].toUpperCase() + part.slice(1) : part))
    .join(" ");
}

const DistributionBars: React.FC<{
  items: CountItem[];
  total: number;
  tone?: "accent" | "danger" | "warning";
}> = ({ items, total, tone = "accent" }) => {
  const max = items.reduce((m, item) => Math.max(m, item.count), 0) || 1;
  const color =
    tone === "danger"
      ? "var(--rf-color-danger)"
      : tone === "warning"
        ? "var(--rf-color-warning)"
        : "var(--rf-color-accent)";
  return (
    <div className={styles.barList}>
      {items.map((item) => (
        <div className={styles.barRow} key={item.key}>
          <span className={styles.barLabel} title={item.key}>
            {titleCase(item.key)}
          </span>
          <div className={styles.barTrack}>
            <div
              className={styles.barFill}
              style={
                {
                  "--bar-pct": `${(item.count / max) * 100}%`,
                  "--bar-color": color,
                } as React.CSSProperties
              }
            />
          </div>
          <span className={styles.barValue}>
            {formatNumber(item.count)}
            {total > 0 && (
              <span className={styles.barPct}>
                {" "}
                {formatRatioPercent(item.count, total)}
              </span>
            )}
          </span>
        </div>
      ))}
    </div>
  );
};

const HourHeatmap: React.FC<{ hours: HourStats[] }> = ({ hours }) => {
  const max = hours.reduce((m, h) => Math.max(m, h.total_calls), 0) || 1;
  return (
    <div className={styles.hourGrid}>
      {hours.map((h) => (
        <div className={styles.hourCol} key={h.hour}>
          <div className={styles.hourBarTrack}>
            <div
              className={styles.hourBarFill}
              title={`${String(h.hour).padStart(2, "0")}:00 — ${formatNumber(
                h.total_calls,
              )} calls, ${formatTokenCount(h.total_tokens)} tokens`}
              style={
                {
                  "--hour-pct": `${(h.total_calls / max) * 100}%`,
                } as React.CSSProperties
              }
            />
          </div>
          {h.hour % 3 === 0 && (
            <span className={styles.hourLabel}>
              {String(h.hour).padStart(2, "0")}
            </span>
          )}
        </div>
      ))}
    </div>
  );
};

export const TasksTab: React.FC<Props> = ({ dateRange }) => {
  const { data, isLoading, isError } = useGetStatsSummaryQuery(
    dateRangeToApiArgs(dateRange),
  );

  if (isLoading) return <Spinner spinning />;
  if (isError) return <ErrorCallout>Failed to load stats</ErrorCallout>;

  const totals = data?.totals;
  const modes = data?.by_mode ?? [];
  const roles = data?.by_task_role ?? [];
  const agents = data?.by_agent ?? [];
  const tasks = data?.by_task ?? [];
  const errors = data?.errors;
  const hours = data?.by_hour ?? [];

  if (!data || (totals?.total_calls ?? 0) === 0) {
    return (
      <p className={styles.emptyText}>
        No agent or task activity yet. Start a task or agent run to see stats!
      </p>
    );
  }

  const totalErrorCalls = errors?.failed_calls ?? 0;
  const totalFinish = (errors?.by_finish_reason ?? []).reduce(
    (sum, item) => sum + item.count,
    0,
  );
  const hasHourData = hours.some((h) => h.total_calls > 0);

  return (
    <div className={styles.root}>
      <StatSection title="Tasks & Agents" icon={ListChecks} dense>
        <StatCard
          icon={Layers}
          title="Tasks"
          value={formatNumber(totals?.total_tasks ?? 0)}
          subtitle="distinct task boards touched"
        />
        <StatCard
          icon={Bot}
          title="Agents"
          value={formatNumber(totals?.total_agents ?? 0)}
          subtitle="distinct agents spawned"
        />
        <StatCard
          icon={GitBranch}
          title="Modes Used"
          value={formatNumber(modes.length)}
          subtitle="distinct chat modes"
        />
        <StatCard
          icon={CheckCircle2}
          tone="success"
          title="Success Rate"
          value={formatRatioPercent(
            totals?.successful_calls ?? 0,
            totals?.total_calls ?? 0,
          )}
          subtitle={`${formatNumber(
            totals?.successful_calls ?? 0,
          )} of ${formatNumber(totals?.total_calls ?? 0)} calls`}
        />
        <StatCard
          icon={ShieldAlert}
          tone={totalErrorCalls > 0 ? "danger" : "muted"}
          title="Failed Calls"
          value={formatNumber(totalErrorCalls)}
          subtitle="across all modes"
        />
        <StatCard
          icon={RefreshCw}
          title="Retried Calls"
          value={formatNumber(totals?.retried_calls ?? 0)}
          subtitle={`${formatNumber(totals?.total_retries ?? 0)} total retries`}
        />
      </StatSection>

      <section className={styles.section}>
        <h3 className={styles.sectionTitle}>
          <Layers size={16} /> By Mode
        </h3>
        {modes.length === 0 ? (
          <p className={styles.emptyText}>No usage data by mode yet.</p>
        ) : (
          <Surface
            animated="rise"
            className={styles.tableWrapper}
            variant="glass"
          >
            <table className={styles.table}>
              <thead>
                <tr>
                  <th className={styles.th}>Mode</th>
                  <th className={styles.th}>Calls</th>
                  <th className={styles.th}>Success</th>
                  <th className={styles.th}>Prompt</th>
                  <th className={styles.th}>Completion</th>
                  <th className={styles.th}>Tokens</th>
                  <th className={styles.th}>Cache</th>
                  <th className={styles.th}>Cost</th>
                  <th className={styles.th}>Avg Duration</th>
                  <th className={styles.th}>Chats</th>
                </tr>
              </thead>
              <tbody className="rf-stagger">
                {modes.map((m) => (
                  <tr key={m.mode} className="rf-enter-rise">
                    <td className={styles.td}>{m.mode}</td>
                    <td className={styles.td}>{formatNumber(m.total_calls)}</td>
                    <td className={styles.td}>
                      {formatRatioPercent(
                        m.successful_calls ?? 0,
                        m.total_calls,
                      )}
                    </td>
                    <td className={styles.td}>
                      {formatTokenCount(m.total_prompt_tokens ?? 0)}
                    </td>
                    <td className={styles.td}>
                      {formatTokenCount(m.total_completion_tokens ?? 0)}
                    </td>
                    <td className={styles.td}>
                      {formatTokenCount(m.total_tokens)}
                    </td>
                    <td className={styles.td}>
                      {formatTokenCount(m.total_cache_read_tokens ?? 0)}
                    </td>
                    <td className={styles.td}>
                      {formatCostDisplay(m.total_cost_usd)}
                    </td>
                    <td className={styles.td}>
                      {formatDuration(m.avg_duration_ms ?? 0)}
                    </td>
                    <td className={styles.td}>
                      {formatNumber(m.conversations ?? 0)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Surface>
        )}
      </section>

      {roles.length > 0 && (
        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>
            <ListChecks size={16} /> By Task Role
          </h3>
          <Surface
            animated="rise"
            className={styles.tableWrapper}
            variant="glass"
          >
            <table className={styles.table}>
              <thead>
                <tr>
                  <th className={styles.th}>Role</th>
                  <th className={styles.th}>Calls</th>
                  <th className={styles.th}>Success</th>
                  <th className={styles.th}>Tokens</th>
                  <th className={styles.th}>Cost</th>
                  <th className={styles.th}>Tasks</th>
                  <th className={styles.th}>Agents</th>
                  <th className={styles.th}>Avg Duration</th>
                </tr>
              </thead>
              <tbody className="rf-stagger">
                {roles.map((r) => (
                  <tr key={r.role} className="rf-enter-rise">
                    <td className={styles.td}>{r.role}</td>
                    <td className={styles.td}>{formatNumber(r.total_calls)}</td>
                    <td className={styles.td}>
                      {formatRatioPercent(r.successful_calls, r.total_calls)}
                    </td>
                    <td className={styles.td}>
                      {formatTokenCount(r.total_tokens)}
                    </td>
                    <td className={styles.td}>
                      {formatCostDisplay(r.total_cost_usd)}
                    </td>
                    <td className={styles.td}>{formatNumber(r.tasks)}</td>
                    <td className={styles.td}>{formatNumber(r.agents)}</td>
                    <td className={styles.td}>
                      {formatDuration(r.avg_duration_ms)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Surface>
        </section>
      )}

      {agents.length > 0 && (
        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>
            <Bot size={16} /> Top Agents
          </h3>
          <Surface
            animated="rise"
            className={styles.tableWrapper}
            variant="glass"
          >
            <table className={styles.table}>
              <thead>
                <tr>
                  <th className={styles.th}>Agent</th>
                  <th className={styles.th}>Mode</th>
                  <th className={styles.th}>Calls</th>
                  <th className={styles.th}>Success</th>
                  <th className={styles.th}>Tokens</th>
                  <th className={styles.th}>Cost</th>
                  <th className={styles.th}>Tasks</th>
                  <th className={styles.th}>Last Active</th>
                </tr>
              </thead>
              <tbody className="rf-stagger">
                {agents.map((a) => (
                  <tr key={a.agent_id} className="rf-enter-rise">
                    <td className={styles.td}>
                      <span className={styles.mono} title={a.agent_id}>
                        {shortId(a.agent_id, 12)}
                      </span>
                    </td>
                    <td className={styles.td}>{a.primary_mode || "—"}</td>
                    <td className={styles.td}>{formatNumber(a.total_calls)}</td>
                    <td className={styles.td}>
                      {formatRatioPercent(a.successful_calls, a.total_calls)}
                    </td>
                    <td className={styles.td}>
                      {formatTokenCount(a.total_tokens)}
                    </td>
                    <td className={styles.td}>
                      {formatCostDisplay(a.total_cost_usd)}
                    </td>
                    <td className={styles.td}>{formatNumber(a.tasks)}</td>
                    <td className={styles.td}>
                      {formatRelativeTime(a.last_active)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Surface>
        </section>
      )}

      {tasks.length > 0 && (
        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>
            <Layers size={16} /> Top Tasks
          </h3>
          <Surface
            animated="rise"
            className={styles.tableWrapper}
            variant="glass"
          >
            <table className={styles.table}>
              <thead>
                <tr>
                  <th className={styles.th}>Task</th>
                  <th className={styles.th}>Calls</th>
                  <th className={styles.th}>Success</th>
                  <th className={styles.th}>Tokens</th>
                  <th className={styles.th}>Cost</th>
                  <th className={styles.th}>Agents</th>
                  <th className={styles.th}>Cards</th>
                  <th className={styles.th}>Last Active</th>
                </tr>
              </thead>
              <tbody className="rf-stagger">
                {tasks.map((task) => (
                  <tr key={task.task_id} className="rf-enter-rise">
                    <td className={styles.td}>
                      <span className={styles.mono} title={task.task_id}>
                        {shortId(task.task_id, 16)}
                      </span>
                    </td>
                    <td className={styles.td}>
                      {formatNumber(task.total_calls)}
                    </td>
                    <td className={styles.td}>
                      {formatRatioPercent(
                        task.successful_calls,
                        task.total_calls,
                      )}
                    </td>
                    <td className={styles.td}>
                      {formatTokenCount(task.total_tokens)}
                    </td>
                    <td className={styles.td}>
                      {formatCostDisplay(task.total_cost_usd)}
                    </td>
                    <td className={styles.td}>{formatNumber(task.agents)}</td>
                    <td className={styles.td}>{formatNumber(task.cards)}</td>
                    <td className={styles.td}>
                      {formatRelativeTime(task.last_active)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Surface>
        </section>
      )}

      {errors &&
        (errors.by_finish_reason.length > 0 ||
          errors.by_category.length > 0) && (
          <section className={styles.section}>
            <h3 className={styles.sectionTitle}>
              <ShieldAlert size={16} /> Reliability
            </h3>
            <div className={styles.reliabilityGrid}>
              {errors.by_finish_reason.length > 0 && (
                <Surface
                  animated="rise"
                  className={styles.panel}
                  variant="glass"
                >
                  <h4 className={styles.panelTitle}>Finish Reasons</h4>
                  <DistributionBars
                    items={errors.by_finish_reason}
                    total={totalFinish}
                    tone="accent"
                  />
                </Surface>
              )}
              {errors.by_category.length > 0 && (
                <Surface
                  animated="rise"
                  className={styles.panel}
                  variant="glass"
                >
                  <h4 className={styles.panelTitle}>Error Categories</h4>
                  <DistributionBars
                    items={errors.by_category}
                    total={totalErrorCalls}
                    tone="danger"
                  />
                </Surface>
              )}
            </div>
          </section>
        )}

      {hasHourData && (
        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>
            <Clock4 size={16} /> Activity by Hour (UTC)
          </h3>
          <Surface animated="rise" className={styles.panel} variant="glass">
            <HourHeatmap hours={hours} />
            <p className={styles.hourCaption}>
              Calls per hour of day · peak{" "}
              {formatCompact(
                hours.reduce((m, h) => Math.max(m, h.total_calls), 0),
              )}{" "}
              calls
            </p>
          </Surface>
        </section>
      )}
    </div>
  );
};
