import React, { useCallback } from "react";
import { Flex, Text, Button } from "@radix-ui/themes";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { useDismissBuddySuggestionMutation, useCreateBuddyConversationMutation } from "../../services/refact/buddy";
import {
  selectBuddySuggestions,
  dismissBuddySuggestion,
} from "./buddySlice";
import { openBuddyChat, newBuddyChatAction } from "../Chat/Thread";
import { push } from "../Pages/pagesSlice";
import { useBuddyState } from "./hooks/useBuddyState";
import { BuddyCanvas } from "./BuddyCanvas";
import { PALETTES } from "./constants";
import type { BuddySuggestion } from "./types";
import styles from "./BuddySuggestionBar.module.css";

interface SuggestionCardProps {
  suggestion: BuddySuggestion;
}

const SuggestionCard: React.FC<SuggestionCardProps> = ({ suggestion }) => {
  const dispatch = useAppDispatch();
  const [dismissMutation] = useDismissBuddySuggestionMutation();
  const [createConversation] = useCreateBuddyConversationMutation();
  const buddy = useBuddyState();
  const palette = PALETTES[buddy.state.paletteIndex] ?? PALETTES[0];

  const handleDismiss = useCallback(async () => {
    await dismissMutation(suggestion.id);
    dispatch(dismissBuddySuggestion(suggestion.id));
  }, [dismissMutation, dispatch, suggestion.id]);

  const handleFixIt = useCallback(async () => {
    await dismissMutation(suggestion.id);
    dispatch(dismissBuddySuggestion(suggestion.id));
    const result = await createConversation(undefined);
    if ("data" in result && result.data) {
      const meta = result.data;
      dispatch(newBuddyChatAction({ chat_id: meta.chat_id }));
      dispatch(openBuddyChat({ chat_id: meta.chat_id, title: meta.title }));
      dispatch(push({ name: "chat" }));
    }
  }, [dismissMutation, createConversation, dispatch, suggestion.id]);

  return (
    <div className={styles.card} style={{ borderColor: palette.body }}>
      <div className={styles.canvasWrap}>
        <BuddyCanvas
          state={buddy.state}
          onEvent={buddy.handleCanvasEvent}
          style={{ width: 48, height: 48 }}
        />
      </div>
      <div className={styles.bubble} style={{ borderColor: palette.body }}>
        <Text size="1" weight="bold" className={styles.title}>
          {suggestion.title}
        </Text>
        <Text size="1" color="gray" className={styles.desc}>
          {suggestion.description}
        </Text>
      </div>
      <Flex gap="1" align="center" className={styles.actions}>
        <Button size="1" variant="soft" onClick={handleFixIt}>
          Fix it →
        </Button>
        <Button size="1" variant="ghost" color="gray" onClick={handleDismiss}>
          Ignore
        </Button>
      </Flex>
    </div>
  );
};

export const BuddySuggestionBar: React.FC = () => {
  const suggestions = useAppSelector(selectBuddySuggestions);
  const active = suggestions.filter((s) => !s.dismissed).slice(0, 1);

  if (active.length === 0) return null;

  return (
    <div className={styles.bar}>
      {active.map((s) => (
        <SuggestionCard key={s.id} suggestion={s} />
      ))}
    </div>
  );
};
