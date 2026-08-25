import { useMemo } from "react";
import { Activity, Database, Sparkles, Wrench } from "lucide-react";

import { DataTable, EmptyState, Surface } from "../../components/ui";
import type { DataTableColumn } from "../../components/ui";
import type {
  PerformanceAggregate,
  PerformanceTelemetryResponse,
} from "../../services/refact/performance";
import { StatSection } from "../StatsDashboard/components/StatSection";
import {
  formatComponentName,
  formatCounters,
  formatCount,
  formatDurationUs,
  formatRatio,
} from "./performanceFormatters";
import styles from "./PerformancePage.module.css";

type PerformanceRow = PerformanceAggregate & {
  id: string;
  label: string;
};

type AggregateTableProps = {
  caption: string;
  emptyMessage: string;
  rows: PerformanceRow[];
};

const aggregateColumns: DataTableColumn<PerformanceRow>[] = [
  {
    id: "component",
    header: "Stage",
    cell: (row) => row.label,
    sortValue: (row) => row.label,
  },
  {
    id: "samples",
    header: "Samples",
    cell: (row) => formatCount(row.sample_count),
    sortValue: (row) => row.sample_count,
    align: "end",
  },
  {
    id: "success",
    header: "Success",
    cell: (row) => formatCount(row.success_count),
    sortValue: (row) => row.success_count,
    align: "end",
  },
  {
    id: "failure",
    header: "Failure",
    cell: (row) => formatCount(row.failure_count),
    sortValue: (row) => row.failure_count,
    align: "end",
  },
  {
    id: "p50",
    header: "p50",
    cell: (row) => formatDurationUs(row.p50_us),
    sortValue: (row) => row.p50_us,
    align: "end",
  },
  {
    id: "p95",
    header: "p95",
    cell: (row) => formatDurationUs(row.p95_us),
    sortValue: (row) => row.p95_us,
    align: "end",
  },
  {
    id: "p99",
    header: "p99",
    cell: (row) => formatDurationUs(row.p99_us),
    sortValue: (row) => row.p99_us,
    align: "end",
  },
  {
    id: "counters",
    header: "Bytes / items",
    cell: (row) => formatCounters(row),
    align: "end",
  },
];

function AggregateTable({ caption, emptyMessage, rows }: AggregateTableProps) {
  return (
    <DataTable
      caption={caption}
      columns={aggregateColumns}
      emptyMessage={emptyMessage}
      enableSorting
      getRowId={(row) => row.id}
      rows={rows}
      wide
    />
  );
}

function asRows(
  components: PerformanceAggregate[],
  predicate: (component: string) => boolean,
): PerformanceRow[] {
  return components
    .filter((component) => predicate(component.component ?? ""))
    .map((component, index) => ({
      ...component,
      id: `${component.component ?? "unknown"}-${index}`,
      label: formatComponentName(component.component),
    }));
}

export function TelemetryPanel({
  telemetry,
}: {
  telemetry: PerformanceTelemetryResponse;
}) {
  const components = useMemo(
    () => telemetry.components ?? [],
    [telemetry.components],
  );
  const sampleCount = components.reduce(
    (count, component) => count + (component.sample_count ?? 0),
    0,
  );
  const groups = useMemo(
    () => ({
      chatAdvancement: asRows(components, (component) =>
        /^(command|stream|sse)\./.test(component),
      ),
      trajectory: asRows(components, (component) =>
        component.startsWith("trajectory."),
      ),
      tools: asRows(components, (component) => component.startsWith("tool.")),
      enrichment: asRows(components, (component) =>
        component.startsWith("enrichment."),
      ),
    }),
    [components],
  );
  const amplification = telemetry.rollups?.index_watcher_vecdb_amplification;
  const watcherRows = amplification?.aggregate
    ? [
        {
          ...amplification.aggregate,
          id: "watcher-vecdb-aggregate",
          label: "Watcher / VecDB aggregate",
        },
      ]
    : [];

  if (sampleCount === 0) {
    return (
      <EmptyState
        icon={Activity}
        title="No telemetry samples yet"
        description="Collection is enabled. Metrics will appear as chats, trajectories, tools, and enrichment stages run."
      />
    );
  }

  return (
    <div className={styles.telemetrySections}>
      <StatSection icon={Activity} title="Chat advancement">
        <AggregateTable
          caption="Chat advancement telemetry"
          emptyMessage="No chat advancement samples yet."
          rows={groups.chatAdvancement}
        />
      </StatSection>
      <StatSection icon={Database} title="Trajectory persistence / index">
        <AggregateTable
          caption="Trajectory persistence and index telemetry"
          emptyMessage="No trajectory persistence or index samples yet."
          rows={groups.trajectory}
        />
      </StatSection>
      <StatSection icon={Database} title="Watcher / VecDB">
        <AggregateTable
          caption="Watcher and VecDB telemetry"
          emptyMessage="No watcher or VecDB samples yet."
          rows={watcherRows}
        />
        {amplification ? (
          <Surface className={styles.amplification} variant="surface-1">
            <dl>
              <div>
                <dt>Index operations / commit</dt>
                <dd>
                  {formatRatio(
                    amplification.trajectory_index_operations_per_commit,
                  )}
                </dd>
              </div>
              <div>
                <dt>Watcher rebuilds / commit</dt>
                <dd>
                  {formatRatio(amplification.watcher_rebuilds_per_commit)}
                </dd>
              </div>
              <div>
                <dt>VecDB searches / enrichment attempt</dt>
                <dd>
                  {formatRatio(
                    amplification.vecdb_searches_per_enrichment_attempt,
                  )}
                </dd>
              </div>
            </dl>
          </Surface>
        ) : null}
      </StatSection>
      <StatSection icon={Wrench} title="Tool stages">
        <AggregateTable
          caption="Tool stage telemetry"
          emptyMessage="No tool stage samples yet."
          rows={groups.tools}
        />
      </StatSection>
      <StatSection icon={Sparkles} title="Automatic enrichment">
        <AggregateTable
          caption="Automatic enrichment telemetry"
          emptyMessage="No automatic enrichment samples yet."
          rows={groups.enrichment}
        />
      </StatSection>
    </div>
  );
}
