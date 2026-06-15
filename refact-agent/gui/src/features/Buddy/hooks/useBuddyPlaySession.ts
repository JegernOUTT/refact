import { useCallback, useEffect, useRef, useState } from "react";
import type { BuddyControl, BuddyPetState } from "../types";
import type {
  BuddySpeechMemory,
  BuddyWorldSpeechCandidate,
} from "../buddySpeech";
import { DIRECTOR_SPEECH_BEATS, pickBuddySpeechLine } from "../buddySpeech";
import type { BuddyWorldIntentKind } from "../buddyWorldDirector";
import {
  BUDDY_REQUEST_PROMPT,
  FETCH_PHASE_DURATIONS_MS,
  FIREFLY_PHASE_DURATIONS_MS,
  GIFT_LINES,
  GIFT_MOMENT_MS,
  PLAY_SESSION_LINES,
  advanceBuddyPlaySession,
  createFetchSession,
  createFireflySession,
  maybeCreateGiftMoment,
  pounceFirefly,
  shouldOfferBuddyRequest,
  throwFetchBall,
  type BuddyGiftMoment,
  type BuddyPlaySession,
} from "../buddyPlaySessions";

export interface BuddyLocalPrompt {
  text: string;
  controls: BuddyControl[];
}

export interface UseBuddyPlaySessionArgs {
  name: string;
  pet: BuddyPetState | undefined;
  busy: boolean;
  offerBusy: boolean;
  buddyX: number;
  directorIntentKind: BuddyWorldIntentKind | null;
  directorIntentStartedAtMs: number;
  speechMemory: BuddySpeechMemory;
}

export interface UseBuddyPlaySessionResult {
  session: BuddyPlaySession | null;
  sessionLine: BuddyWorldSpeechCandidate | null;
  gift: BuddyGiftMoment | null;
  requestPrompt: BuddyLocalPrompt | null;
  startFetch: () => void;
  startFirefly: () => void;
  handleSceneClick: (xPercent: number, yPercent: number) => boolean;
  handleLocalControl: (control: BuddyControl) => boolean;
  cancelPlay: () => void;
}

const REQUEST_CHECK_MS = 30_000;
const GIFT_FALLBACK_PAYOFF_MS = 14_000;

export const BUDDY_REQUEST_CONTROLS: BuddyControl[] = [
  {
    id: "buddy-request-fetch",
    label: "Throw the ball 🎾",
    action: "local_fetch",
    style: "primary",
  },
  {
    id: "buddy-request-later",
    label: "Later",
    action: "local_dismiss",
    style: "secondary",
  },
];

export function useBuddyPlaySession(
  args: UseBuddyPlaySessionArgs,
): UseBuddyPlaySessionResult {
  const {
    name,
    pet,
    busy,
    offerBusy,
    buddyX,
    directorIntentKind,
    directorIntentStartedAtMs,
    speechMemory,
  } = args;
  const [session, setSession] = useState<BuddyPlaySession | null>(null);
  const [sessionLine, setSessionLine] =
    useState<BuddyWorldSpeechCandidate | null>(null);
  const [gift, setGift] = useState<BuddyGiftMoment | null>(null);
  const [requestPrompt, setRequestPrompt] = useState<BuddyLocalPrompt | null>(
    null,
  );
  const buddyXRef = useRef(buddyX);
  const lastOfferAtMsRef = useRef(0);
  const offersThisSessionRef = useRef(0);

  useEffect(() => {
    buddyXRef.current = buddyX;
  }, [buddyX]);

  const speakPhase = useCallback(
    (key: string) => {
      const pool = PLAY_SESSION_LINES[key];
      if (!pool) return;
      setSessionLine({
        text: pickBuddySpeechLine(speechMemory, pool.poolKey, pool.lines, name),
        style: pool.style,
      });
    },
    [name, speechMemory],
  );

  const cancelPlay = useCallback(() => {
    setSession(null);
    setSessionLine(null);
    setGift(null);
    setRequestPrompt(null);
  }, []);

  const startFetch = useCallback(() => {
    setRequestPrompt(null);
    setGift(null);
    setSession(createFetchSession(Date.now()));
    speakPhase("fetch:armed");
  }, [speakPhase]);

  const startFirefly = useCallback(() => {
    setRequestPrompt(null);
    setGift(null);
    setSession(createFireflySession(Date.now()));
    speakPhase("firefly:armed");
  }, [speakPhase]);

  useEffect(() => {
    if (busy && (session || requestPrompt)) {
      setSession(null);
      setSessionLine(null);
      setRequestPrompt(null);
    }
  }, [busy, session, requestPrompt]);

  useEffect(() => {
    if (!session) return;
    const durations =
      session.kind === "fetch"
        ? FETCH_PHASE_DURATIONS_MS
        : FIREFLY_PHASE_DURATIONS_MS;
    const duration = durations[session.phase as keyof typeof durations];
    const elapsed = Date.now() - session.phaseStartedAtMs;
    const timer = window.setTimeout(
      () => {
        setSession((current) => {
          if (!current || current.phase !== session.phase) return current;
          const settledNow = Math.max(
            Date.now(),
            current.phaseStartedAtMs + duration,
          );
          const advanced = advanceBuddyPlaySession(current, settledNow);
          if (!advanced) {
            setSessionLine(null);
            return null;
          }
          if (advanced.phase !== current.phase) {
            if (advanced.kind === "firefly" && advanced.phase === "resolved") {
              speakPhase(
                advanced.caughtLast ? "firefly:caught" : "firefly:missed",
              );
            } else {
              speakPhase(`${advanced.kind}:${advanced.phase}`);
            }
          }
          return advanced;
        });
      },
      Math.max(16, duration - elapsed),
    );
    return () => window.clearTimeout(timer);
  }, [session, speakPhase]);

  const handleSceneClick = useCallback(
    (xPercent: number, yPercent: number): boolean => {
      if (!session || session.phase !== "armed") return false;
      const dispatched =
        session.kind === "fetch"
          ? throwFetchBall(
              session,
              xPercent,
              yPercent,
              buddyXRef.current,
              Date.now(),
            )
          : pounceFirefly(session, xPercent, yPercent, Date.now());
      setSession((current) =>
        current &&
        current.kind === session.kind &&
        current.phase === "armed" &&
        current.armedAtMs === session.armedAtMs
          ? dispatched
          : current,
      );
      speakPhase(
        session.kind === "firefly" ? "firefly:pouncing" : "fetch:throwing",
      );
      return true;
    },
    [session, speakPhase],
  );

  const handleLocalControl = useCallback(
    (control: BuddyControl): boolean => {
      if (control.action === "local_fetch") {
        offersThisSessionRef.current += 1;
        lastOfferAtMsRef.current = Date.now();
        startFetch();
        return true;
      }
      if (control.action === "local_dismiss") {
        offersThisSessionRef.current += 1;
        lastOfferAtMsRef.current = Date.now();
        setRequestPrompt(null);
        return true;
      }
      return false;
    },
    [startFetch],
  );

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (session || requestPrompt || gift) return;
      if (
        !shouldOfferBuddyRequest({
          pet,
          nowMs: Date.now(),
          lastOfferAtMs: lastOfferAtMsRef.current,
          offersThisSession: offersThisSessionRef.current,
          busy: offerBusy,
        })
      ) {
        return;
      }
      setRequestPrompt({
        text: pickBuddySpeechLine(
          speechMemory,
          BUDDY_REQUEST_PROMPT.poolKey,
          BUDDY_REQUEST_PROMPT.lines,
          name,
        ),
        controls: BUDDY_REQUEST_CONTROLS,
      });
    }, REQUEST_CHECK_MS);
    return () => window.clearInterval(timer);
  }, [gift, offerBusy, name, pet, requestPrompt, session, speechMemory]);

  useEffect(() => {
    if (!directorIntentKind || directorIntentStartedAtMs <= 0) return;
    if (session) return;
    const beats = DIRECTOR_SPEECH_BEATS[directorIntentKind];
    const payoffAtMs =
      beats && beats.length > 0
        ? beats[beats.length - 1].atMs + 700
        : GIFT_FALLBACK_PAYOFF_MS;
    const timer = window.setTimeout(
      () => {
        const created = maybeCreateGiftMoment(
          directorIntentKind,
          Math.floor(directorIntentStartedAtMs / 1000),
          Date.now(),
        );
        if (!created) return;
        setGift(created);
        const pool = GIFT_LINES[created.item];
        setSessionLine({
          text: pickBuddySpeechLine(
            speechMemory,
            pool.poolKey,
            pool.lines,
            name,
          ),
          style: pool.style,
        });
      },
      Math.max(0, directorIntentStartedAtMs + payoffAtMs - Date.now()),
    );
    return () => window.clearTimeout(timer);
  }, [
    directorIntentKind,
    directorIntentStartedAtMs,
    name,
    session,
    speechMemory,
  ]);

  useEffect(() => {
    if (!gift) return;
    const timer = window.setTimeout(
      () => {
        setGift(null);
        setSessionLine(null);
      },
      Math.max(0, gift.startedAtMs + GIFT_MOMENT_MS - Date.now()),
    );
    return () => window.clearTimeout(timer);
  }, [gift]);

  return {
    session,
    sessionLine: session || gift || requestPrompt ? sessionLine : null,
    gift,
    requestPrompt,
    startFetch,
    startFirefly,
    handleSceneClick,
    handleLocalControl,
    cancelPlay,
  };
}
