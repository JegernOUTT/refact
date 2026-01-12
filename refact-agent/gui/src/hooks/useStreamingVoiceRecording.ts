import { useState, useRef, useCallback, useEffect } from "react";
import {
  subscribeToVoiceStream,
  sendVoiceChunk,
  VoiceStreamEvent,
} from "../services/refact/voice";

export interface UseStreamingVoiceRecordingResult {
  isRecording: boolean;
  isFinishing: boolean;
  transcript: string;
  error: string | null;
  startRecording: () => Promise<void>;
  stopRecording: () => Promise<string>;
  cancelRecording: () => void;
}

function floatTo16BitPCM(samples: Float32Array): ArrayBuffer {
  const buffer = new ArrayBuffer(samples.length * 2);
  const view = new DataView(buffer);
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true);
  }
  return buffer;
}

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.byteLength; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

export function useStreamingVoiceRecording(): UseStreamingVoiceRecordingResult {
  const [isRecording, setIsRecording] = useState(false);
  const [isFinishing, setIsFinishing] = useState(false);
  const [transcript, setTranscript] = useState("");
  const [error, setError] = useState<string | null>(null);

  const sessionIdRef = useRef<string>("");
  const streamRef = useRef<MediaStream | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const processorRef = useRef<ScriptProcessorNode | null>(null);
  const unsubscribeRef = useRef<(() => void) | null>(null);
  const bufferRef = useRef<Float32Array[]>([]);
  const sendIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const finalizeResolveRef = useRef<((text: string) => void) | null>(null);
  const finalizeRejectRef = useRef<((err: Error) => void) | null>(null);
  const recordingStartTimeRef = useRef<number>(0);

  const LIVE_WINDOW_SECONDS = 20;

  const cleanupStream = useCallback(() => {
    if (streamRef.current) {
      streamRef.current.getTracks().forEach((track) => track.stop());
      streamRef.current = null;
    }
  }, []);

  const handleEvent = useCallback(
    (event: VoiceStreamEvent) => {
      if (event.type === "transcript") {
        const elapsedSeconds =
          (Date.now() - recordingStartTimeRef.current) / 1000;
        const wasCut = !event.is_final && elapsedSeconds > LIVE_WINDOW_SECONDS;
        setTranscript(wasCut ? "... " + event.text : event.text);
        if (event.is_final) {
          setIsFinishing(false);
          finalizeResolveRef.current?.(event.text);
          finalizeResolveRef.current = null;
          finalizeRejectRef.current = null;
          unsubscribeRef.current?.();
          unsubscribeRef.current = null;
          cleanupStream();
        }
      } else if (event.type === "error") {
        setError(event.message);
        setIsFinishing(false);
        finalizeRejectRef.current?.(new Error(event.message));
        finalizeResolveRef.current = null;
        finalizeRejectRef.current = null;
        unsubscribeRef.current?.();
        unsubscribeRef.current = null;
        cleanupStream();
      } else {
        // event.type === "ended"
        setIsFinishing(false);
        finalizeRejectRef.current?.(
          new Error("Stream ended without final transcript"),
        );
        finalizeResolveRef.current = null;
        finalizeRejectRef.current = null;
        unsubscribeRef.current?.();
        unsubscribeRef.current = null;
        cleanupStream();
      }
    },
    [cleanupStream],
  );

  const sendBufferedAudio = useCallback(async (final: boolean) => {
    const hasAudio = bufferRef.current.length > 0;

    if (!hasAudio && !final) return;

    let base64 = "";

    if (hasAudio) {
      const totalLength = bufferRef.current.reduce(
        (acc, arr) => acc + arr.length,
        0,
      );
      const combined = new Float32Array(totalLength);
      let offset = 0;
      for (const arr of bufferRef.current) {
        combined.set(arr, offset);
        offset += arr.length;
      }

      if (!final) bufferRef.current = [];

      const pcmBuffer = floatTo16BitPCM(combined);
      base64 = arrayBufferToBase64(pcmBuffer);
    }

    await sendVoiceChunk(sessionIdRef.current, base64, final);
  }, []);

  const startRecording = useCallback(async () => {
    setError(null);
    setTranscript("");
    setIsFinishing(false);
    finalizeResolveRef.current = null;
    finalizeRejectRef.current = null;
    bufferRef.current = [];

    sessionIdRef.current = crypto.randomUUID();
    recordingStartTimeRef.current = Date.now();

    unsubscribeRef.current = subscribeToVoiceStream(
      sessionIdRef.current,
      undefined,
      handleEvent,
      (err) => {
        setError(err.message);
        setIsFinishing(false);
        setIsRecording(false);
        finalizeRejectRef.current?.(err);
        finalizeResolveRef.current = null;
        finalizeRejectRef.current = null;
        cleanupStream();
      },
    );

    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    streamRef.current = stream;

    const audioContext = new AudioContext({ sampleRate: 16000 });
    audioContextRef.current = audioContext;

    const source = audioContext.createMediaStreamSource(stream);
    const processor = audioContext.createScriptProcessor(4096, 1, 1);
    processorRef.current = processor;

    processor.onaudioprocess = (e) => {
      const inputData = e.inputBuffer.getChannelData(0);
      bufferRef.current.push(new Float32Array(inputData));
    };

    source.connect(processor);
    processor.connect(audioContext.destination);

    sendIntervalRef.current = setInterval(() => {
      void sendBufferedAudio(false);
    }, 1000);

    setIsRecording(true);
  }, [handleEvent, sendBufferedAudio, cleanupStream]);

  const stopRecording = useCallback(async (): Promise<string> => {
    if (!isRecording) throw new Error("Not recording");

    setIsRecording(false);
    setIsFinishing(true);

    const finalPromise = new Promise<string>((resolve, reject) => {
      finalizeResolveRef.current = resolve;
      finalizeRejectRef.current = reject;
    });

    if (sendIntervalRef.current) {
      clearInterval(sendIntervalRef.current);
      sendIntervalRef.current = null;
    }

    if (processorRef.current) {
      processorRef.current.disconnect();
      processorRef.current = null;
    }

    if (audioContextRef.current) {
      await audioContextRef.current.close();
      audioContextRef.current = null;
    }

    if (streamRef.current) {
      streamRef.current.getTracks().forEach((track) => track.stop());
      streamRef.current = null;
    }

    await sendBufferedAudio(true);
    bufferRef.current = [];

    return finalPromise;
  }, [isRecording, sendBufferedAudio]);

  const cancelRecording = useCallback(() => {
    if (!isRecording && !isFinishing) return;

    setIsRecording(false);
    setIsFinishing(false);
    setTranscript("");

    finalizeResolveRef.current = null;
    finalizeRejectRef.current = null;

    if (sendIntervalRef.current) {
      clearInterval(sendIntervalRef.current);
      sendIntervalRef.current = null;
    }

    if (processorRef.current) {
      processorRef.current.disconnect();
      processorRef.current = null;
    }

    if (audioContextRef.current) {
      void audioContextRef.current.close();
      audioContextRef.current = null;
    }

    unsubscribeRef.current?.();
    unsubscribeRef.current = null;

    cleanupStream();
    bufferRef.current = [];
  }, [isRecording, isFinishing, cleanupStream]);

  useEffect(() => {
    return () => {
      unsubscribeRef.current?.();
      if (sendIntervalRef.current) clearInterval(sendIntervalRef.current);
      if (processorRef.current) {
        processorRef.current.disconnect();
        processorRef.current = null;
      }
      if (audioContextRef.current) {
        void audioContextRef.current.close();
        audioContextRef.current = null;
      }
      if (streamRef.current) {
        streamRef.current.getTracks().forEach((track) => track.stop());
        streamRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (!isRecording) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.repeat) return;

      const el = event.target as HTMLElement;
      if (
        el.tagName === "INPUT" ||
        el.tagName === "TEXTAREA" ||
        el.isContentEditable
      ) {
        return;
      }

      if (event.key === "Enter") {
        event.preventDefault();
        void stopRecording();
      } else if (event.key === "Escape") {
        event.preventDefault();
        cancelRecording();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isRecording, stopRecording, cancelRecording]);

  return {
    isRecording,
    isFinishing,
    transcript,
    error,
    startRecording,
    stopRecording,
    cancelRecording,
  };
}
