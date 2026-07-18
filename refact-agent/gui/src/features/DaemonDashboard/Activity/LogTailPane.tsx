import { useEffect, useRef, useState } from "react";

import { Badge, Button, Select, Surface } from "../../../components/ui";
import { useConfig } from "../../../hooks";
import {
  resolveDaemonLogsUrl,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import { appendLogLine, MAX_LOG_LINES, mergeLogLines } from "./activityState";
import styles from "./ActivityPage.module.css";

type LogTailPaneProps = {
  projectIdParam?: string;
  workers: DaemonWorker[];
};

export function LogTailPane({ projectIdParam, workers }: LogTailPaneProps) {
  const config = useConfig();
  const [target, setTarget] = useState(projectIdParam ?? "daemon");
  const [lines, setLines] = useState<string[]>([]);
  const [paused, setPaused] = useState(false);
  const [streamStatus, setStreamStatus] = useState<
    "connecting" | "connected" | "reconnecting"
  >("connecting");
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);
  const pausedRef = useRef(paused);
  const pausedLinesRef = useRef<string[]>([]);
  const outputRef = useRef<HTMLPreElement>(null);
  const projectId = target === "daemon" ? null : target;
  const selectedWorker = workers.find(
    (worker) => worker.project_id === projectId,
  );

  useEffect(() => {
    if (projectIdParam) setTarget(projectIdParam);
  }, [projectIdParam]);

  useEffect(() => {
    pausedRef.current = paused;
    if (paused) return;
    if (pausedLinesRef.current.length > 0) {
      const pendingLines = pausedLinesRef.current;
      pausedLinesRef.current = [];
      setLines((current) => mergeLogLines(current, pendingLines));
    }
  }, [paused]);

  useEffect(() => {
    setLines([]);
    pausedLinesRef.current = [];
    setStreamStatus("connecting");
    const source = new EventSource(
      resolveDaemonLogsUrl(config, projectId, true, MAX_LOG_LINES),
    );
    source.onopen = () => setStreamStatus("connected");
    source.onmessage = (message: MessageEvent<string>) => {
      const line = message.data;
      if (pausedRef.current) {
        pausedLinesRef.current = appendLogLine(
          pausedLinesRef.current,
          line,
          false,
        );
        return;
      }
      setLines((current) => appendLogLine(current, line, false));
    };
    source.onerror = () => setStreamStatus("reconnecting");
    return () => source.close();
  }, [config, projectId]);

  useEffect(() => {
    if (paused) return;
    outputRef.current?.scrollTo({ top: outputRef.current.scrollHeight });
  }, [lines, paused]);

  async function downloadLog() {
    setDownloading(true);
    setDownloadError(null);
    try {
      const response = await fetch(
        resolveDaemonLogsUrl(config, projectId, false, 10_000),
      );
      if (!response.ok) throw new Error(`Download failed (${response.status})`);
      const blob = await response.blob();
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = projectId
        ? `worker-${selectedWorker?.slug ?? projectId}.log`
        : "daemon.log";
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
    } catch (error) {
      setDownloadError(
        error instanceof Error ? error.message : "Download failed",
      );
    } finally {
      setDownloading(false);
    }
  }

  const statusTone =
    streamStatus === "connected"
      ? "success"
      : streamStatus === "reconnecting"
        ? "warning"
        : "muted";

  return (
    <Surface className={styles.logPane} variant="glass" radius="card">
      <div className={styles.paneHeader}>
        <div>
          <h2>Log tail</h2>
          <p>{lines.length} retained lines</p>
        </div>
        <Badge tone={statusTone} variant="soft">
          {streamStatus}
        </Badge>
      </div>
      <div className={styles.logControls}>
        <Select value={target} onValueChange={setTarget}>
          <Select.Trigger aria-label="Choose log source">
            <Select.Value />
          </Select.Trigger>
          <Select.Content>
            <Select.Item value="daemon">Daemon log</Select.Item>
            {workers.map((worker) => (
              <Select.Item key={worker.project_id} value={worker.project_id}>
                {worker.slug}
              </Select.Item>
            ))}
          </Select.Content>
        </Select>
        <Button
          aria-pressed={paused}
          onClick={() => setPaused((current) => !current)}
          size="sm"
          variant={paused ? "soft" : "ghost"}
        >
          {paused ? "Resume" : "Pause"}
        </Button>
        <Button
          onClick={() => {
            pausedLinesRef.current = [];
            setLines([]);
          }}
          size="sm"
          variant="ghost"
        >
          Clear
        </Button>
        <Button
          loading={downloading}
          onClick={() => void downloadLog()}
          size="sm"
          variant="ghost"
        >
          Download
        </Button>
      </div>
      {downloadError ? (
        <p className={styles.downloadError} role="alert">
          {downloadError}
        </p>
      ) : null}
      <div className={`${styles.logOutput} scrollX`}>
        <pre ref={outputRef} aria-live={paused ? "off" : "polite"}>
          {lines.length > 0 ? lines.join("\n") : "Waiting for log output…"}
        </pre>
      </div>
    </Surface>
  );
}
