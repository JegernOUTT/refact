import { buildApiUrl } from "./apiUrl";
import { normalizeConnection, type PortOrConnection } from "./chatCommands";

function voiceUrl(
  connection: PortOrConnection,
  path: string,
  query?: Record<string, string | number | boolean | null | undefined>,
): string {
  return buildApiUrl(normalizeConnection(connection), path, query);
}

export interface TranscribeRequest {
  audio_data: string;
  mime_type?: string;
  language?: string;
}

export interface TranscribeResponse {
  text: string;
  language: string;
  duration_ms: number;
}

export interface VoiceStatusResponse {
  enabled: boolean;
  model_loaded: boolean;
  model_name: string;
  is_downloading: boolean;
  download_progress: number;
}

export interface DownloadModelRequest {
  model?: string;
}

export interface DownloadModelResponse {
  success: boolean;
  message: string;
}

export async function transcribeAudio(
  connection: PortOrConnection,
  request: TranscribeRequest,
): Promise<TranscribeResponse> {
  const response = await fetch(voiceUrl(connection, "/v1/voice/transcribe"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(request),
  });

  if (!response.ok) {
    const error = await response.text();
    throw new Error(error || "Transcription failed");
  }

  return response.json() as Promise<TranscribeResponse>;
}

export async function getVoiceStatus(
  connection: PortOrConnection,
): Promise<VoiceStatusResponse> {
  const response = await fetch(voiceUrl(connection, "/v1/voice/status"));
  if (!response.ok) {
    throw new Error("Failed to get voice status");
  }
  return response.json() as Promise<VoiceStatusResponse>;
}

export async function downloadVoiceModel(
  connection: PortOrConnection,
  model?: string,
): Promise<DownloadModelResponse> {
  const response = await fetch(voiceUrl(connection, "/v1/voice/download"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ model: model ?? "base.en" }),
  });

  if (!response.ok) {
    const error = await response.text();
    throw new Error(error || "Download failed");
  }

  return response.json() as Promise<DownloadModelResponse>;
}

export interface StreamingTranscriptEvent {
  type: "transcript";
  session_id: string;
  text: string;
  is_final: boolean;
  duration_ms: number;
}

export interface StreamingErrorEvent {
  type: "error";
  message: string;
}

export interface StreamingEndedEvent {
  type: "ended";
}

export type VoiceStreamEvent =
  | StreamingTranscriptEvent
  | StreamingErrorEvent
  | StreamingEndedEvent;

export function subscribeToVoiceStream(
  connection: PortOrConnection,
  sessionId: string,
  language: string | undefined,
  onEvent: (event: VoiceStreamEvent) => void,
  onError?: (error: Error) => void,
): () => void {
  const url = voiceUrl(
    connection,
    `/v1/voice/stream/${encodeURIComponent(sessionId)}/subscribe`,
    language ? { language } : undefined,
  );

  const eventSource = new EventSource(url);

  eventSource.onmessage = (e) => {
    const event = JSON.parse(e.data as string) as VoiceStreamEvent;
    onEvent(event);
    if (event.type === "ended") {
      eventSource.close();
    }
  };

  eventSource.onerror = () => {
    onError?.(new Error("Stream connection error"));
    eventSource.close();
  };

  return () => eventSource.close();
}

export async function sendVoiceChunk(
  connection: PortOrConnection,
  sessionId: string,
  audioData: string,
  isFinal: boolean,
  language?: string,
): Promise<void> {
  const response = await fetch(
    voiceUrl(
      connection,
      `/v1/voice/stream/${encodeURIComponent(sessionId)}/chunk`,
    ),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        audio_data: audioData,
        is_final: isFinal,
        language,
      }),
    },
  );

  if (!response.ok) {
    const error = await response.text();
    throw new Error(error || "Failed to send chunk");
  }
}
