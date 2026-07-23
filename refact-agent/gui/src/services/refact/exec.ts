import { buildApiUrl } from "./apiUrl";
import { normalizeConnection, type PortOrConnection } from "./chatCommands";

export type ExecStatus =
  | "starting"
  | "running"
  | "exited"
  | "failed"
  | "killed"
  | "timed_out";

export type ExecSpawnRequest = {
  command?: string;
  argv?: string[];
  cwd?: string;
  env?: Record<string, string>;
  pty?: boolean;
  rows?: number;
  cols?: number;
  service_name?: string;
};

export type ExecSpawnResponse = {
  process_id: string;
  status: ExecStatus;
  command_preview?: string;
};

export type ExecProcessSnapshot = {
  process_id: string;
  status: ExecStatus;
  command_preview: string;
  created_at_ms: number;
  tty: boolean;
  service_name: string | null;
};

export type ExecListResponse = {
  processes: ExecProcessSnapshot[];
};

export type ExecOutputChunk = {
  seq: number;
  stream: "stdout" | "stderr" | "combined";
  text: string;
  offset?: number;
};

export type ExecReadResponse = {
  chunks: ExecOutputChunk[];
  next_seq: number;
  status: ExecStatus;
};

export type ExecKillResponse = {
  process_id: string;
  status: ExecStatus;
};

export type ExecStdinResponse = {
  process_id: string;
  status: ExecStatus;
  bytes_written: number;
  since_seq: number;
  next_seq: number;
  latest_seq: number;
};

export type ExecSnapshotEvent = {
  status: ExecStatus;
  chunks: ExecOutputChunk[];
  next_seq: number;
};

export type ExecExitEvent = {
  process_id: string;
  status: ExecStatus;
};

export class ExecHttpError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "ExecHttpError";
    this.status = status;
  }
}

function execUrl(
  connection: PortOrConnection,
  path: string,
  query?: Record<string, string | number | boolean | null | undefined>,
): string {
  return buildApiUrl(normalizeConnection(connection), path, query);
}

function execHeaders(apiKey?: string): Record<string, string> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (apiKey) headers.Authorization = `Bearer ${apiKey}`;
  return headers;
}

async function execRequest<T>(
  connection: PortOrConnection,
  path: string,
  apiKey: string | undefined,
  init?: RequestInit,
  query?: Record<string, string | number | boolean | null | undefined>,
): Promise<T> {
  const response = await fetch(execUrl(connection, path, query), {
    ...init,
    headers: {
      ...execHeaders(apiKey),
      ...init?.headers,
    },
  });
  if (!response.ok) {
    const detail = (await response.text()).trim();
    throw new ExecHttpError(
      detail || `Exec request failed: ${response.status}`,
      response.status,
    );
  }
  return (await response.json()) as T;
}

export function spawnExec(
  request: ExecSpawnRequest,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<ExecSpawnResponse> {
  return execRequest(connection, "/v1/exec/spawn", apiKey, {
    method: "POST",
    body: JSON.stringify(request),
  });
}

export function listExec(
  connection: PortOrConnection,
  apiKey?: string,
): Promise<ExecListResponse> {
  return execRequest(connection, "/v1/exec/list", apiKey);
}

export function readExec(
  processId: string,
  sinceSeq: number,
  connection: PortOrConnection,
  apiKey?: string,
  raw = false,
): Promise<ExecReadResponse> {
  return execRequest(
    connection,
    `/v1/exec/${encodeURIComponent(processId)}/read`,
    apiKey,
    undefined,
    { since_seq: sinceSeq, limit: 10_000, raw: raw ? true : undefined },
  );
}

export function killExec(
  processId: string,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<ExecKillResponse> {
  return execRequest(
    connection,
    `/v1/exec/${encodeURIComponent(processId)}/kill`,
    apiKey,
    { method: "POST", body: "{}" },
  );
}

export function resizeExec(
  processId: string,
  rows: number,
  cols: number,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<Record<string, never>> {
  return execRequest(
    connection,
    `/v1/exec/${encodeURIComponent(processId)}/resize`,
    apiKey,
    {
      method: "POST",
      body: JSON.stringify({ rows, cols }),
    },
  );
}

export function writeProcessStdin(
  processId: string,
  chars: string,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<ExecStdinResponse> {
  return execRequest(
    connection,
    `/v1/exec/${encodeURIComponent(processId)}/stdin`,
    apiKey,
    {
      method: "POST",
      body: JSON.stringify({ chars }),
    },
  );
}

export function execSubscribeUrl(
  processId: string,
  connection: PortOrConnection,
  sinceSeq = 0,
): string {
  return execUrl(
    connection,
    `/v1/exec/${encodeURIComponent(processId)}/subscribe`,
    { since_seq: sinceSeq },
  );
}
