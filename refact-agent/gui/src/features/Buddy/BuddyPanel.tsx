import React, { useCallback, useMemo } from "react";
import { Text, Button } from "@radix-ui/themes";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import { openBuddyChat, newBuddyChatAction } from "../Chat/Thread";
import { BuddyCanvas } from "./BuddyCanvas";
import { useBuddyState } from "./hooks/useBuddyState";
import {
  selectBuddySnapshot,
  selectIsBuddyEnabled,
} from "./buddySlice";
import { PALETTES, STAGES } from "./constants";
import { useCreateBuddyConversationMutation } from "../../services/refact/buddy";
import { useGetSetupStatusQuery } from "../../services/refact/setupStatus";
import styles from "./BuddyPanel.module.css";

export const BuddyPanel: React.FC = () => {
  const dispatch = useAppDispatch();
  const snapshot = useAppSelector(selectBuddySnapshot);
  const enabled = useAppSelector(selectIsBuddyEnabled);
  const [createConversation] = useCreateBuddyConversationMutation();
  const { data: setupData } = useGetSetupStatusQuery(undefined, {
    refetchOnMountOrArgChange: true,
  });
  const setupNeeded = setupData !== undefined && !setupData.configured;

  const buddy = useBuddyState();
  const { state } = buddy;

  const paletteIndex = snapshot?.settings.palette_index ?? state.paletteIndex;
  const palette = PALETTES[paletteIndex] ?? PALETTES[0];

  const progression = snapshot?.state.progression;
  const identity = snapshot?.state.identity;
  const semantic = snapshot?.state.semantic;

  const stageIdx = progression?.stage ?? state.progress.stage;
  const stage = STAGES[stageIdx] ?? STAGES[0];
  const nextStage = STAGES[stageIdx + 1];

  const xp = progression?.xp ?? state.progress.xp;
  const xpNext = progression?.xp_next ?? nextStage?.xpThreshold;

  const xpFill = useMemo(() => {
    const currentXp = progression?.xp ?? 0;
    const nextXp = progression?.xp_next ?? 100;
    if (nextXp <= 0) return 100;
    return Math.min(100, Math.max(0, (currentXp / nextXp) * 100));
  }, [progression]);

  const name = identity?.name ?? state.name;
  const statusText = semantic?.headline ?? stage.tagline;

  const handleOpen = useCallback(() => {
    dispatch(push({ name: "buddy" }));
  }, [dispatch]);

  const handleNewChat = useCallback(async () => {
    const result = await createConversation(undefined);
    if ("data" in result && result.data) {
      const meta = result.data;
      dispatch(newBuddyChatAction({ chat_id: meta.chat_id }));
      dispatch(openBuddyChat({ chat_id: meta.chat_id, title: meta.title }));
      dispatch(push({ name: "chat" }));
    }
  }, [createConversation, dispatch]);

  if (!enabled && snapshot !== null) return null;

  return (
    <div className={styles.block}>
      <div className={styles.body}>
        <div className={styles.glowWrap}>
          <div
            className={styles.glow}
            style={{ backgroundColor: palette.body }}
          />
          <BuddyCanvas
            state={state}
            onEvent={buddy.handleCanvasEvent}
            style={{ width: 280, height: 280 }}
          />
        </div>

        <div className={styles.info}>
          <div className={styles.nameRow}>
            <Text size="2" weight="bold">
              {name}
            </Text>
            <span
              className={styles.stageBadge}
              style={{
                backgroundColor: palette.body + "33",
                color: palette.body,
              }}
            >
              {stage.emoji} {stage.name}
            </span>
          </div>

          {statusText && (
            <div className={styles.statusText}>{statusText}</div>
          )}

          <div className={styles.xpRow}>
            <span className={styles.xpLabel}>{xp} XP</span>
            <div className={styles.xpBar}>
              <div
                className={styles.xpFill}
                style={{ width: `${xpFill}%` }}
              />
            </div>
            {xpNext && (
              <span className={styles.xpLabel}>{xpNext}</span>
            )}
          </div>

          <div className={styles.actions}>
            <Button size="1" variant="soft" onClick={handleOpen}>
              Open →
            </Button>
            <Button size="1" variant="soft" onClick={handleNewChat}>
              New Chat
            </Button>
            {setupNeeded && (
              <span className={styles.setupChip}>⚙ Setup needed</span>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
