const VOICE_API_BASE = "/v1/voice";

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
  request: TranscribeRequest,
): Promise<TranscribeResponse> {
  const response = await fetch(`${VOICE_API_BASE}/transcribe`, {
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

export async function getVoiceStatus(): Promise<VoiceStatusResponse> {
  const response = await fetch(`${VOICE_API_BASE}/status`);
  if (!response.ok) {
    throw new Error("Failed to get voice status");
  }
  return response.json() as Promise<VoiceStatusResponse>;
}

export async function downloadVoiceModel(
  model?: string,
): Promise<DownloadModelResponse> {
  const response = await fetch(`${VOICE_API_BASE}/download`, {
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
  sessionId: string,
  language: string | undefined,
  onEvent: (event: VoiceStreamEvent) => void,
  onError?: (error: Error) => void,
): () => void {
  const params = new URLSearchParams();
  if (language) params.set("language", language);
  const url = `${VOICE_API_BASE}/stream/${sessionId}/subscribe?${params.toString()}`;

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
  sessionId: string,
  audioData: string,
  isFinal: boolean,
  language?: string,
): Promise<void> {
  const response = await fetch(`${VOICE_API_BASE}/stream/${sessionId}/chunk`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      audio_data: audioData,
      is_final: isFinal,
      language,
    }),
  });

  if (!response.ok) {
    const error = await response.text();
    throw new Error(error || "Failed to send chunk");
  }
}
