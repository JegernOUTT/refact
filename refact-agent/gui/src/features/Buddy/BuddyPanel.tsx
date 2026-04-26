import React, { useCallback, useMemo } from "react";
import { Button, Tooltip } from "@radix-ui/themes";
import { ExternalLinkIcon, ChatBubbleIcon } from "@radix-ui/react-icons";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import { openBuddyChat, newBuddyChatAction } from "../Chat/Thread";
import { BuddyCanvas } from "./BuddyCanvas";
import { BuddySpeechCloud } from "./BuddySpeechCloud";
import { BuddyRecentChats } from "./BuddyRecentChats";
import { useBuddyState } from "./hooks/useBuddyState";
import {
  selectBuddySnapshot,
  selectIsBuddyEnabled,
  selectNowPlaying,
} from "./buddySlice";
import { PALETTES, STAGES, SIGNALS } from "./constants";
import { computeXpFill } from "./buddyUtils";
import { useCreateBuddyConversationMutation } from "../../services/refact/buddy";
import styles from "./BuddyPanel.module.css";

export const BuddyPanel: React.FC = () => {
  const dispatch = useAppDispatch();
  const snapshot = useAppSelector(selectBuddySnapshot);
  const enabled = useAppSelector(selectIsBuddyEnabled);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const [createConversation] = useCreateBuddyConversationMutation();

  const buddy = useBuddyState();
  const { state } = buddy;

  const paletteIndex = snapshot?.state.identity.palette_index ?? state.paletteIndex;
  const palette = PALETTES[paletteIndex] ?? PALETTES[0];

  const progression = snapshot?.state.progression;
  const identity = snapshot?.state.identity;

  const stageIdx = progression?.stage ?? state.progress.stage;
  const stage = STAGES[stageIdx] ?? STAGES[0];

  const xp = progression?.xp ?? state.progress.xp;

  const xpFill = useMemo(
    () => computeXpFill(progression?.xp ?? 0, progression?.xp_next ?? 100),
    [progression],
  );

  const name = identity?.name ?? state.name;

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

  if (snapshot === null) return null;
  if (!enabled) return null;

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
            displaySize={200}
          />
        </div>

        <BuddySpeechCloud />

        <div className={styles.info}>
          <div className={styles.nameRow}>
            <span className={styles.name}>{name}</span>
            <span
              className={styles.stageBadge}
              style={{
                backgroundColor: palette.body + "33",
                color: palette.body,
              }}
            >
              {stage.emoji} {stage.name}
            </span>
            <div className={styles.xpBarInline}>
              <div
                className={styles.xpFillInline}
                style={{ width: `${xpFill}%` }}
              />
            </div>
            <span className={styles.xpText}>{xp}</span>
          </div>

          {nowPlaying && (
            <div className={styles.statusBubble}>
              <span className={styles.statusIcon}>
                {SIGNALS[nowPlaying.signal_type]?.icon ?? "⚡"}
              </span>
              <span className={styles.statusTitle}>{nowPlaying.title}</span>
              {nowPlaying.progress != null && (
                <div className={styles.progressBar}>
                  <div style={{ width: `${nowPlaying.progress}%` }} />
                </div>
              )}
            </div>
          )}

          <div className={styles.actions}>
            <Tooltip content="Open Buddy">
              <Button size="1" variant="ghost" onClick={handleOpen}>
                <ExternalLinkIcon width={12} height={12} />
              </Button>
            </Tooltip>
            <Tooltip content="New Chat">
              <Button size="1" variant="ghost" onClick={handleNewChat}>
                <ChatBubbleIcon width={12} height={12} />
              </Button>
            </Tooltip>
          </div>
        </div>
      </div>
      <div className={styles.recentChats}>
        <BuddyRecentChats compact maxItems={4} showFilters={false} onViewAll={handleOpen} />
      </div>
    </div>
  );
};
