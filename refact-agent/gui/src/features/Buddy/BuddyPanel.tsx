import {
  isLegacyWorkControl,
  useBuddyLegacyControls,
} from "./hooks/useBuddyLegacyControls";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Button, Text } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import { BuddyCanvas } from "./BuddyCanvas";
import { useBuddyState } from "./hooks/useBuddyState";
import { useBuddyOpportunities } from "./hooks/useBuddyOpportunities";
import {
  selectBuddySnapshot,
  selectIsBuddyInteractiveEnabled,
  selectNowPlaying,
  selectActiveSpeech,
  selectBuddyDiagnostics,
  dismissRuntimeEvent,
} from "./buddySlice";
import type { BuddyControl } from "./types";
import { getSignalDef, PALETTES } from "./constants";
import { buddyActionResultMessage } from "./hooks/useExecuteBuddyAction";
import { useBuddyControlAction } from "./hooks/useBuddyControlAction";
import {
  getOpportunityActionFromControl,
  getOpportunityActionIndexFromControl,
  getOpportunityDismissAction,
  opportunityActionControls,
  opportunitySpeechText,
} from "./buddyOpportunityActions";
import {
  useDismissBuddyRuntimeEventMutation,
  useUpdateBuddySettingsMutation,
} from "../../services/refact/buddy";
import styles from "./BuddyPanel.module.css";
import { requiresEditorialClaim } from "./useBuddyEditorialDelivery";

export const BuddyPanel: React.FC = () => {
  const dispatch = useAppDispatch();
  const legacyControls = useBuddyLegacyControls();
  const snapshot = useAppSelector(selectBuddySnapshot);
  const enabled = useAppSelector(selectIsBuddyInteractiveEnabled);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const rawSpeech = useAppSelector(selectActiveSpeech);
  const activeSpeech = requiresEditorialClaim(rawSpeech) ? null : rawSpeech;
  const diagnostics = useAppSelector(selectBuddyDiagnostics);
  const { unread } = useBuddyOpportunities();
  const [opportunityIndex, setOpportunityIndex] = useState(0);
  const [dismissedOpportunityIds, setDismissedOpportunityIds] = useState<
    Set<string>
  >(new Set());
  const [opportunityError, setOpportunityError] = useState<string | null>(null);
  const { control: runControl, opportunity: runOpportunity } =
    useBuddyControlAction();
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();
  const [updateSettings, { isLoading: isEnabling }] =
    useUpdateBuddySettingsMutation();

  const buddy = useBuddyState();
  const { state } = buddy;

  const activeDiagnostic = activeSpeech?.chat_id
    ? diagnostics.find((diag) => diag.chat_id === activeSpeech.chat_id)
    : undefined;
  const activeRuntime =
    nowPlaying?.dismissed || requiresEditorialClaim(nowPlaying)
      ? null
      : nowPlaying;
  const activeRuntimeSignal = activeRuntime
    ? getSignalDef(activeRuntime.signal_type)
    : null;
  const runtimeDiagnostic = activeRuntime?.chat_id
    ? diagnostics.find((diag) => diag.chat_id === activeRuntime.chat_id)
    : undefined;

  const paletteIndex =
    snapshot?.state.identity.palette_index ?? state.paletteIndex;
  const palette = PALETTES[paletteIndex] ?? PALETTES[0];

  const activeOpportunities = useMemo(
    () =>
      unread.filter(
        (opp) => !dismissedOpportunityIds.has(`opportunity-${opp.id}`),
      ),
    [dismissedOpportunityIds, unread],
  );

  useEffect(() => {
    if (activeOpportunities.length <= 1) return;
    const timer = window.setInterval(() => {
      setOpportunityIndex((index) => (index + 1) % activeOpportunities.length);
    }, 12_000);
    return () => window.clearInterval(timer);
  }, [activeOpportunities.length]);

  useEffect(() => {
    if (opportunityIndex < activeOpportunities.length) return;
    setOpportunityIndex(0);
  }, [activeOpportunities.length, opportunityIndex]);

  const topOpportunity =
    activeOpportunities.length > 0
      ? activeOpportunities[opportunityIndex % activeOpportunities.length]
      : null;
  useEffect(() => {
    setOpportunityError(null);
  }, [topOpportunity?.id]);
  const speechText = opportunityError
    ? opportunityError
    : activeSpeech
      ? activeSpeech.text
      : topOpportunity
        ? opportunitySpeechText(topOpportunity)
        : activeRuntime?.speech_text ?? activeRuntime?.title ?? null;
  const speechControls = activeSpeech
    ? activeSpeech.controls
    : topOpportunity
      ? opportunityActionControls(topOpportunity)
      : activeRuntime?.controls?.length
        ? activeRuntime.controls
        : undefined;
  const speechHandler = activeSpeech
    ? async (ctrl: BuddyControl) => {
        const result = await runControl(
          ctrl,
          {
            triggerText: activeSpeech.text,
            triggerSource: "runtime",
            sourceChatId: activeSpeech.chat_id,
            diagnostic: activeDiagnostic,
          },
          "panel-speech",
        );
        if (result.status === "ignored") return;
        setOpportunityError(buddyActionResultMessage(result));
      }
    : topOpportunity
      ? async (ctrl: BuddyControl) => {
          const actionIndex = getOpportunityActionIndexFromControl(ctrl);
          if (actionIndex == null) return;
          const action = getOpportunityActionFromControl(ctrl, topOpportunity);
          if (!action) return;

          setOpportunityError(null);
          const isDismiss = action.kind === "dismiss";
          const dismissAction = isDismiss
            ? getOpportunityDismissAction(topOpportunity)
            : null;
          const result = await runOpportunity(
            dismissAction ? dismissAction.action : action,
            topOpportunity,
            dismissAction ? dismissAction.actionIndex : actionIndex,
            undefined,
            "panel-opportunity",
          );
          if (result.status === "ignored") return;
          if (result.status !== "success") {
            setOpportunityError(buddyActionResultMessage(result));
            return;
          }
          setDismissedOpportunityIds((prev) =>
            new Set(prev).add(`opportunity-${topOpportunity.id}`),
          );
          if (isDismiss) {
            setOpportunityIndex(0);
          } else {
            setOpportunityIndex((index) => index + 1);
          }
        }
      : activeRuntime?.controls?.length
        ? async (ctrl: BuddyControl) => {
            if (
              ctrl.action === "dismiss" ||
              ctrl.action === "dismiss_speech" ||
              ctrl.action === "dismiss_runtime_event"
            ) {
              dispatch(dismissRuntimeEvent(activeRuntime.id));
              try {
                await dismissRuntimeMutation(activeRuntime.id).unwrap();
              } catch {
                // Local dismiss is enough to hide the dashboard bubble immediately.
              }
              return;
            }

            const result = await runControl(
              ctrl,
              {
                triggerText: activeRuntime.speech_text ?? activeRuntime.title,
                triggerSource: "runtime",
                sourceChatId: activeRuntime.chat_id,
                diagnostic: runtimeDiagnostic,
              },
              "panel-runtime",
            );
            if (result.status === "ignored") return;
            setOpportunityError(buddyActionResultMessage(result));
          }
        : undefined;

  const handleOpen = useCallback(() => {
    dispatch(push({ name: "buddy" }));
  }, [dispatch]);

  const handleEnable = useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      void updateSettings({ enabled: true });
    },
    [updateSettings],
  );

  if (snapshot === null) return null;
  if (!enabled) {
    return (
      <div
        className={styles.disabledBlock}
        data-testid="buddy-panel-disabled"
        onClick={handleOpen}
      >
        <Text size="2" weight="bold">
          Pixel is disabled
        </Text>
        <Text size="1" color="gray" align="center">
          Buddy is paused, not gone. Tiny gremlin standby mode.
        </Text>
        <Button size="1" onClick={handleEnable} disabled={isEnabling}>
          Enable
        </Button>
      </div>
    );
  }

  return (
    <div
      className={styles.block}
      onClick={handleOpen}
      style={{ cursor: "pointer" }}
    >
      <div className={styles.body}>
        <div className={styles.scene}>
          <div className={styles.glowWrap} onClick={(e) => e.stopPropagation()}>
            <div
              className={styles.glow}
              style={{ backgroundColor: palette.body }}
            />
            <BuddyCanvas
              state={state}
              onEvent={buddy.handleCanvasEvent}
              displaySize={200}
              speechOverride={speechText}
              speechControls={speechControls?.filter(
                (control) => legacyControls || !isLegacyWorkControl(control),
              )}
              speechIntent={activeSpeech?.speech_intent}
              onSpeechControlClick={speechHandler}
            />
          </div>
        </div>

        <div className={styles.info}>
          {activeRuntime?.progress != null && (
            <div className={styles.statusBubble}>
              <span className={styles.statusIcon}>
                {activeRuntimeSignal?.icon}
              </span>
              <div className={styles.progressBar}>
                <div style={{ width: `${activeRuntime.progress}%` }} />
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
