import { buildApiUrl } from "./apiUrl";
import { normalizeConnection, type PortOrConnection } from "./chatCommands";

export type ExecStdinResponse = {
  process_id: string;
  status: string;
  bytes_written: number;
  since_seq: number;
  next_seq: number;
  latest_seq: number;
};

export async function writeProcessStdin(
  processId: string,
  chars: string,
  connection: PortOrConnection,
  apiKey?: string,
): Promise<ExecStdinResponse> {
  const url = buildApiUrl(
    normalizeConnection(connection),
    `/v1/exec/${encodeURIComponent(processId)}/stdin`,
  );
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (apiKey) {
    headers.Authorization = `Bearer ${apiKey}`;
  }

  const response = await fetch(url, {
    method: "POST",
    headers,
    body: JSON.stringify({ chars }),
  });
  if (!response.ok) {
    throw new Error(`Failed to write stdin: ${response.status}`);
  }

  return (await response.json()) as ExecStdinResponse;
}
