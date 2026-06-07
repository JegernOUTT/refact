export interface UseVoiceRecordingResult {
    isRecording: boolean;
    isProcessing: boolean;
    error: string | null;
    startRecording: () => Promise<void>;
    stopRecording: () => Promise<Blob | null>;
    toggleRecording: () => Promise<Blob | null>;
}
export declare function useVoiceRecording(): UseVoiceRecordingResult;
