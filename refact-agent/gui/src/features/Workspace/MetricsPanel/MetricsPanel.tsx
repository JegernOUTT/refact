import { useEffect } from "react";
import { Activity, RefreshCw } from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
} from "../../../components/ui";
import {
  metricsApi,
  useGetMetricsQuery,
} from "../../../services/refact/metrics";
import {
  sampleLabel,
  seriesLabel,
  type MetricEntry,
  type MetricExplanation,
  type MetricSeries,
  type MetricsPage,
} from "../../../services/refact/metricsTypes";
import { humanizeIdentifier } from "../../../utils/displayNames";
import { panelErrorDescription } from "../panelError";
import { useAppDispatch } from "../../../hooks";
import { usePanelScope } from "../usePanelScope";
import styles from "./MetricsPanel.module.css";

function SeriesItem({ series }: { series: MetricSeries }) {
  return (
    <li className={styles.item}>
      <div className={styles.heading}>
        <strong className="rf-wrap-anywhere">{seriesLabel(series)}</strong>
        <Badge tone="muted">{series.sample.kind}</Badge>
      </div>
      <span className={styles.muted}>{sampleLabel(series.sample)}</span>
    </li>
  );
}

function ExplanationItem({ explanation }: { explanation: MetricExplanation }) {
  return (
    <li className={styles.item}>
      <div className={styles.heading}>
        <strong>{humanizeIdentifier(explanation.decision)}</strong>
        <Badge tone="muted">{explanation.outcome}</Badge>
      </div>
      <p className={styles.text}>Rule: {explanation.rule}</p>
      <dl
        className={styles.inputs}
        aria-label={`Inputs for ${explanation.rule}`}
      >
        {explanation.inputs.map(([name, value]) => (
          <div key={`${name}:${value}`}>
            <dt>{name}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
    </li>
  );
}

function Entry({ entry }: { entry: MetricEntry }) {
  if (entry.kind === "series") return <SeriesItem series={entry.series} />;
  return <ExplanationItem explanation={entry.explanation} />;
}

function Summary({ page }: { page: MetricsPage }) {
  return (
    <p className={styles.muted} role="status">
      {page.series_total} series · {page.explanations_total} explanations
      {page.truncated ? " · latest page is truncated" : ""}
    </p>
  );
}

export function MetricsPanel() {
  const endpoint = usePanelScope();
  return <MetricsContents key={endpoint} endpoint={endpoint} />;
}

function MetricsContents({ endpoint }: { endpoint: string }) {
  const dispatch = useAppDispatch();
  const {
    currentData: page,
    isFetching,
    error,
    refetch,
  } = useGetMetricsQuery({ endpoint });
  useEffect(
    () => () => {
      dispatch(
        metricsApi.util.getRunningQueryThunk("getMetrics", { endpoint }),
      )?.abort();
    },
    [dispatch, endpoint],
  );
  useEffect(() => {
    if (!error) return;
    dispatch(
      metricsApi.util.updateQueryData("getMetrics", { endpoint }, (draft) => {
        draft.entries = [];
        draft.total = 0;
        draft.returned = 0;
        draft.series_total = 0;
        draft.explanations_total = 0;
        delete draft.next_cursor;
      }),
    );
  }, [error, dispatch, endpoint]);
  return (
    <section className={styles.panel} aria-label="Metrics and explanations">
      <header className={styles.heading}>
        <h2>Metrics</h2>
        <IconButton
          aria-label="Refresh metrics"
          icon={RefreshCw}
          variant="ghost"
          size="sm"
          onClick={() => void refetch()}
          disabled={isFetching}
        />
      </header>
      <p className={styles.muted}>
        Current bounded counters and why automated decisions fired. Values are
        read-only and redacted before they leave the engine.
      </p>
      {error ? (
        <ErrorState
          title="Metrics unavailable"
          description={panelErrorDescription(error)}
          retry={<Button onClick={() => void refetch()}>Refresh</Button>}
        />
      ) : !page || isFetching ? (
        <LoadingState label="Loading metrics" />
      ) : (
        <>
          <Summary page={page} />
          <div className={styles.body}>
            {page.entries.length === 0 ? (
              <EmptyState
                icon={Activity}
                title="No metrics recorded yet"
                description="This engine has not recorded a counter or an automatic decision yet."
                variant="compact"
              />
            ) : (
              <ul className={styles.list} aria-label="Metrics entries">
                {page.entries.map((entry, index) => (
                  <Entry
                    key={
                      entry.kind === "series"
                        ? seriesLabel(entry.series)
                        : `${entry.explanation.seq}:${index}`
                    }
                    entry={entry}
                  />
                ))}
              </ul>
            )}
          </div>
        </>
      )}
    </section>
  );
}
