export interface UseVoiceInputResult {
    isRecording: boolean;
    isFinishing: boolean;
    isVoiceActive: boolean;
    isDownloading: boolean;
    downloadProgress: number;
    error: string | null;
    voiceEnabled: boolean;
    modelLoaded: boolean;
    liveTranscript: string;
    toggleRecording: () => Promise<string | null>;
    cancelRecording: () => void;
}
export declare function useVoiceInput(onTranscript: (text: string) => void): UseVoiceInputResult;
