import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import { BuddyCanvas } from "./BuddyCanvas";
import { useBuddyState } from "./hooks/useBuddyState";
import { useBuddyOpportunities } from "./hooks/useBuddyOpportunities";
import {
  selectBuddySnapshot,
  selectIsBuddyEnabled,
  selectNowPlaying,
  selectActiveSpeech,
  selectBuddyDiagnostics,
  dismissRuntimeEvent,
} from "./buddySlice";
import { executeBuddyAction } from "./executeBuddyAction";
import type { BuddyControl } from "./types";
import { PALETTES, SIGNALS } from "./constants";
import { useExecuteBuddyAction } from "./hooks/useExecuteBuddyAction";
import {
  getOpportunityActionFromControl,
  getOpportunityActionIndexFromControl,
  opportunityActionControls,
  opportunitySpeechText,
} from "./buddyOpportunityActions";
import { useDismissBuddyRuntimeEventMutation } from "../../services/refact/buddy";
import styles from "./BuddyPanel.module.css";

export const BuddyPanel: React.FC = () => {
  const dispatch = useAppDispatch();
  const snapshot = useAppSelector(selectBuddySnapshot);
  const enabled = useAppSelector(selectIsBuddyEnabled);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const activeSpeech = useAppSelector(selectActiveSpeech);
  const diagnostics = useAppSelector(selectBuddyDiagnostics);
  const { unread } = useBuddyOpportunities();
  const [opportunityIndex, setOpportunityIndex] = useState(0);
  const [dismissedOpportunityIds, setDismissedOpportunityIds] = useState<
    Set<string>
  >(new Set());
  const executeOpportunityAction = useExecuteBuddyAction();
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();

  const buddy = useBuddyState();
  const { state } = buddy;

  const activeDiagnostic = activeSpeech?.chat_id
    ? diagnostics.find((diag) => diag.chat_id === activeSpeech.chat_id)
    : undefined;
  const activeRuntime = nowPlaying?.dismissed ? null : nowPlaying;
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
  const speechText = activeSpeech
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
        await executeBuddyAction(ctrl, dispatch, {
          triggerText: activeSpeech.text,
          triggerSource: "runtime",
          sourceChatId: activeSpeech.chat_id,
          diagnostic: activeDiagnostic,
        });
      }
    : topOpportunity
      ? async (ctrl: BuddyControl) => {
          const actionIndex = getOpportunityActionIndexFromControl(ctrl);
          if (actionIndex == null) return;
          const action = getOpportunityActionFromControl(ctrl, topOpportunity);
          if (!action) return;

          if (action.kind === "dismiss") {
            setDismissedOpportunityIds((prev) => {
              const next = new Set(prev);
              for (const opp of activeOpportunities) {
                next.add(`opportunity-${opp.id}`);
              }
              return next;
            });
            await Promise.all(
              activeOpportunities.map((opp) =>
                executeOpportunityAction(action, opp, actionIndex),
              ),
            );
            setOpportunityIndex(0);
            return;
          }

          await executeOpportunityAction(action, topOpportunity, actionIndex);
          setDismissedOpportunityIds((prev) =>
            new Set(prev).add(`opportunity-${topOpportunity.id}`),
          );
          setOpportunityIndex((index) => index + 1);
        }
      : activeRuntime?.controls?.length
        ? async (ctrl: BuddyControl) => {
            if (ctrl.action === "dismiss" || ctrl.action === "dismiss_speech") {
              dispatch(dismissRuntimeEvent(activeRuntime.id));
              try {
                await dismissRuntimeMutation(activeRuntime.id).unwrap();
              } catch {
                // Local dismiss is enough to hide the dashboard bubble immediately.
              }
              return;
            }

            await executeBuddyAction(ctrl, dispatch, {
              triggerText: activeRuntime.speech_text ?? activeRuntime.title,
              triggerSource: "runtime",
              sourceChatId: activeRuntime.chat_id,
              diagnostic: runtimeDiagnostic,
            });
          }
        : undefined;

  const handleOpen = useCallback(() => {
    dispatch(push({ name: "buddy" }));
  }, [dispatch]);

  if (snapshot === null) return null;
  if (!enabled) return null;

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
              speechControls={speechControls}
              onSpeechControlClick={speechHandler}
            />
          </div>
        </div>

        <div className={styles.info}>
          {activeRuntime?.progress != null && (
            <div className={styles.statusBubble}>
              <span className={styles.statusIcon}>
                {SIGNALS[activeRuntime.signal_type].icon}
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
