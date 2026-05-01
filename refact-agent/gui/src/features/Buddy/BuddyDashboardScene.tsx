import React, { useCallback, useState } from "react";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { openChatInModeAndStart } from "../Chat/Thread";
import { useGetSetupStatusQuery } from "../../services/refact/setupStatus";
import {
  selectActiveSpeech,
  selectBuddyDiagnostics,
  selectBuddySuggestions,
  selectBuddyLoaded,
  selectBuddySnapshot,
  selectIsBuddyEnabled,
  selectNowPlaying,
  selectPulse,
  selectRuntimeQueue,
  dismissRuntimeEvent,
} from "./buddySlice";
import { PALETTES, STAGES } from "./constants";
import {
  executeBuddyAction,
  navigateFromBuddyPage,
} from "./executeBuddyAction";
import { useBuddyState } from "./hooks/useBuddyState";
import { BuddyWorld } from "./BuddyWorld";
import {
  buildBuddySceneSpeech,
  buildBuddySceneSpeechCandidates,
  pickBuddySceneSpeechCandidate,
} from "./buddySceneSpeech";
import { useDismissBuddyRuntimeEventMutation } from "../../services/refact/buddy";
import type { BuddyCareAction, BuddyControl, BuddyPage } from "./types";

export const BuddyDashboardScene: React.FC = () => {
  const dispatch = useAppDispatch();
  const snapshot = useAppSelector(selectBuddySnapshot);
  const loaded = useAppSelector(selectBuddyLoaded);
  const enabled = useAppSelector(selectIsBuddyEnabled);
  const pulse = useAppSelector(selectPulse);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const runtimeQueue = useAppSelector(selectRuntimeQueue);
  const activeSpeech = useAppSelector(selectActiveSpeech);
  const suggestions = useAppSelector(selectBuddySuggestions);
  const diagnostics = useAppSelector(selectBuddyDiagnostics);
  const [setupDismissed, setSetupDismissed] = useState(false);
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();
  const buddy = useBuddyState();
  const { state } = buddy;
  const { data: setupData } = useGetSetupStatusQuery(undefined, {
    refetchOnMountOrArgChange: true,
  });

  const setupNeeded = !setupData?.configured && !setupDismissed;
  const progression = snapshot?.state.progression;
  const pet = snapshot?.state.pet;
  const activeQuest = snapshot?.state.active_quest ?? null;
  const stage = STAGES[progression?.stage ?? state.progress.stage] ?? STAGES[0];
  const paletteIndex =
    snapshot?.state.identity.palette_index ?? state.paletteIndex;
  const palette = PALETTES[paletteIndex] ?? PALETTES[0];

  const activeSuggestion =
    suggestions.find((suggestion) => !suggestion.dismissed) ?? null;
  const sceneSpeech = activeSpeech
    ? buildBuddySceneSpeech({
        activeSpeech,
        nowPlaying,
        runtimeQueue,
        activeSuggestion,
      })
    : pickBuddySceneSpeechCandidate(
        buildBuddySceneSpeechCandidates({
          nowPlaying,
          runtimeQueue,
          activeSuggestion,
        }),
      );
  const activeDiagnostic = sceneSpeech?.chat_id
    ? diagnostics.find((diag) => diag.chat_id === sceneSpeech.chat_id)
    : undefined;

  const dismissRuntimeSpeech = useCallback(
    async (runtimeEventId: string) => {
      dispatch(dismissRuntimeEvent(runtimeEventId));
      await dismissRuntimeMutation(runtimeEventId).unwrap();
    },
    [dispatch, dismissRuntimeMutation],
  );

  const handleCare = useCallback(
    async (action: BuddyCareAction, toy?: string) => {
      await executeBuddyAction(
        {
          id: `scene-care-${action}`,
          label: action,
          action: `care_${action}`,
          action_param: toy,
          style: "primary",
        },
        dispatch,
      );
    },
    [dispatch],
  );

  const handleOpenPage = useCallback(
    (page: BuddyPage) => {
      navigateFromBuddyPage(page, dispatch);
    },
    [dispatch],
  );

  const handleRunMode = useCallback(
    (mode: string) => {
      void dispatch(openChatInModeAndStart({ mode }));
    },
    [dispatch],
  );

  const handleSpeechControl = useCallback(
    async (control: BuddyControl) => {
      if (!sceneSpeech) return;
      if (
        sceneSpeech.source === "runtime" &&
        sceneSpeech.runtimeEventId &&
        (control.action === "dismiss" || control.action === "dismiss_speech")
      ) {
        await dismissRuntimeSpeech(sceneSpeech.runtimeEventId);
        return;
      }
      await executeBuddyAction(control, dispatch, {
        triggerText: sceneSpeech.text,
        triggerSource:
          sceneSpeech.source === "suggestion" ? "suggestion" : "runtime",
        sourceChatId: sceneSpeech.chat_id,
        diagnostic: activeDiagnostic,
      });
    },
    [activeDiagnostic, dismissRuntimeSpeech, dispatch, sceneSpeech],
  );

  if (!loaded || snapshot === null || !enabled) {
    return null;
  }

  return (
    <BuddyWorld
      compact
      palette={palette}
      stage={stage}
      state={state}
      pulse={pulse}
      pet={pet}
      nowPlaying={nowPlaying}
      activeQuest={activeQuest}
      activeSpeech={sceneSpeech}
      setupNeeded={setupNeeded}
      onCanvasEvent={buddy.handleCanvasEvent}
      onCare={(action, toy) => void handleCare(action, toy)}
      onOpenPage={handleOpenPage}
      onRunMode={handleRunMode}
      onDismissSetup={() => setSetupDismissed(true)}
      onSpeechControl={(control) => void handleSpeechControl(control)}
    />
  );
};
