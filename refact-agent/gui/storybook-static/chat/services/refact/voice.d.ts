import { type PortOrConnection } from "./chatCommands";
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
export declare function transcribeAudio(connection: PortOrConnection, request: TranscribeRequest): Promise<TranscribeResponse>;
export declare function getVoiceStatus(connection: PortOrConnection): Promise<VoiceStatusResponse>;
export declare function downloadVoiceModel(connection: PortOrConnection, model?: string): Promise<DownloadModelResponse>;
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
export type VoiceStreamEvent = StreamingTranscriptEvent | StreamingErrorEvent | StreamingEndedEvent;
export declare function subscribeToVoiceStream(connection: PortOrConnection, sessionId: string, language: string | undefined, onEvent: (event: VoiceStreamEvent) => void, onError?: (error: Error) => void): () => void;
export declare function sendVoiceChunk(connection: PortOrConnection, sessionId: string, audioData: string, isFinal: boolean, language?: string): Promise<void>;
