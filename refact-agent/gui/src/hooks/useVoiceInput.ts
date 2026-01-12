import { useState, useCallback, useEffect } from "react";
import { useStreamingVoiceRecording } from "./useStreamingVoiceRecording";
import {
  getVoiceStatus,
  downloadVoiceModel,
  VoiceStatusResponse,
} from "../services/refact/voice";

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

export function useVoiceInput(
  onTranscript: (text: string) => void,
): UseVoiceInputResult {
  const {
    isRecording,
    isFinishing,
    transcript,
    error: recordingError,
    startRecording,
    stopRecording,
    cancelRecording,
  } = useStreamingVoiceRecording();
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<VoiceStatusResponse | null>(null);

  useEffect(() => {
    if (recordingError) {
      setError(recordingError);
    }
  }, [recordingError]);

  useEffect(() => {
    getVoiceStatus()
      .then(setStatus)
      .catch(() => setStatus(null));
  }, []);

  useEffect(() => {
    if (!status?.is_downloading) return;

    const interval = setInterval(() => {
      getVoiceStatus()
        .then(setStatus)
        .catch(() => {
          // Silently ignore errors during polling
        });
    }, 1000);

    return () => clearInterval(interval);
  }, [status?.is_downloading]);

  const toggleRecording = useCallback(async (): Promise<string | null> => {
    setError(null);

    if (isRecording) {
      try {
        const finalText = await stopRecording();
        const trimmed = finalText.trim();
        if (trimmed) {
          onTranscript(trimmed);
          return trimmed;
        }
        return null;
      } catch (err) {
        const message =
          err instanceof Error ? err.message : "Failed to get transcript";
        setError(message);
        return null;
      }
    } else {
      try {
        await startRecording();
      } catch (err) {
        const message =
          err instanceof Error ? err.message : "Failed to start recording";
        if (message.includes("Model not downloaded")) {
          downloadVoiceModel().catch(() => {
            // Silently ignore download errors
          });
          const newStatus = await getVoiceStatus().catch(() => null);
          if (newStatus) setStatus(newStatus);
        }
        setError(message);
      }
      return null;
    }
  }, [isRecording, startRecording, stopRecording, onTranscript]);

  return {
    isRecording,
    isFinishing,
    isVoiceActive: isRecording || isFinishing,
    isDownloading: status?.is_downloading ?? false,
    downloadProgress: status?.download_progress ?? 0,
    error,
    voiceEnabled: status?.enabled ?? false,
    modelLoaded: status?.model_loaded ?? false,
    liveTranscript: transcript,
    toggleRecording,
    cancelRecording,
  };
}
