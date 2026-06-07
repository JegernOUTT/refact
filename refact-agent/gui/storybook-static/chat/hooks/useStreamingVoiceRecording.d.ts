import type { EngineApiConnection } from "../services/refact/chatCommands";
export interface UseStreamingVoiceRecordingResult {
    isRecording: boolean;
    isFinishing: boolean;
    transcript: string;
    error: string | null;
    startRecording: () => Promise<void>;
    stopRecording: () => Promise<string>;
    cancelRecording: () => void;
}
export declare function useStreamingVoiceRecording(connection: EngineApiConnection): UseStreamingVoiceRecordingResult;
