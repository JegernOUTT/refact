import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
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
import { getSignalDef, PALETTES } from "./constants";
import {
  formatOpportunityActionError,
  useExecuteBuddyAction,
} from "./hooks/useExecuteBuddyAction";
import {
  getOpportunityActionFromControl,
  getOpportunityActionIndexFromControl,
  getOpportunityDismissAction,
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
  const [opportunityError, setOpportunityError] = useState<string | null>(null);
  const pendingOpportunityRef = useRef(false);
  const executeOpportunityAction = useExecuteBuddyAction();
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();

  const buddy = useBuddyState();
  const { state } = buddy;

  const activeDiagnostic = activeSpeech?.chat_id
    ? diagnostics.find((diag) => diag.chat_id === activeSpeech.chat_id)
    : undefined;
  const activeRuntime = nowPlaying?.dismissed ? null : nowPlaying;
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
        await executeBuddyAction(ctrl, dispatch, {
          triggerText: activeSpeech.text,
          triggerSource: "runtime",
          sourceChatId: activeSpeech.chat_id,
          diagnostic: activeDiagnostic,
        });
      }
    : topOpportunity
      ? async (ctrl: BuddyControl) => {
          if (pendingOpportunityRef.current) return;
          const actionIndex = getOpportunityActionIndexFromControl(ctrl);
          if (actionIndex == null) return;
          const action = getOpportunityActionFromControl(ctrl, topOpportunity);
          if (!action) return;

          pendingOpportunityRef.current = true;
          setOpportunityError(null);
          try {
            if (action.kind === "dismiss") {
              const results = await Promise.allSettled(
                activeOpportunities.map(async (opp) => {
                  const dismissAction = getOpportunityDismissAction(opp);
                  await executeOpportunityAction(
                    dismissAction.action,
                    opp,
                    dismissAction.actionIndex,
                  );
                  return opp.id;
                }),
              );
              const dismissedIds = results.flatMap((result) =>
                result.status === "fulfilled" ? [result.value] : [],
              );
              if (dismissedIds.length > 0) {
                setDismissedOpportunityIds((prev) => {
                  const next = new Set(prev);
                  for (const oppId of dismissedIds) {
                    next.add(`opportunity-${oppId}`);
                  }
                  return next;
                });
              }
              const failed = results.find(
                (result) => result.status === "rejected",
              );
              if (failed) {
                setOpportunityError(
                  formatOpportunityActionError(failed.reason),
                );
              }
              setOpportunityIndex(0);
              return;
            }

            await executeOpportunityAction(action, topOpportunity, actionIndex);
            setDismissedOpportunityIds((prev) =>
              new Set(prev).add(`opportunity-${topOpportunity.id}`),
            );
            setOpportunityIndex((index) => index + 1);
          } catch (error) {
            setOpportunityError(formatOpportunityActionError(error));
          } finally {
            pendingOpportunityRef.current = false;
          }
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
