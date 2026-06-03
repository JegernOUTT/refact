import React, { useCallback } from "react";
import { Flex, Text, Button } from "@radix-ui/themes";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { useDismissBuddySuggestionMutation } from "../../services/refact/buddy";
import { selectBuddySuggestions, dismissBuddySuggestion } from "./buddySlice";
import { startBuddyInvestigation } from "../Chat/Thread";
import { useBuddyState } from "./hooks/useBuddyState";
import { BuddyCanvas } from "./BuddyCanvas";
import { PALETTES } from "./constants";
import type { BuddyControl, BuddySuggestion } from "./types";
import { executeBuddyAction } from "./executeBuddyAction";
import styles from "./BuddySuggestionBar.module.css";

interface SuggestionCardProps {
  suggestion: BuddySuggestion;
}

const SuggestionCard: React.FC<SuggestionCardProps> = ({ suggestion }) => {
  const dispatch = useAppDispatch();
  const [dismissMutation] = useDismissBuddySuggestionMutation();
  const buddy = useBuddyState();
  const palette = PALETTES[buddy.state.paletteIndex] ?? PALETTES[0];

  const handleDismiss = useCallback(async () => {
    await dismissMutation(suggestion.id);
    dispatch(dismissBuddySuggestion(suggestion.id));
  }, [dismissMutation, dispatch, suggestion.id]);

  const handleInvestigate = useCallback(async () => {
    await dismissMutation(suggestion.id);
    dispatch(dismissBuddySuggestion(suggestion.id));
    await dispatch(
      startBuddyInvestigation({
        triggerText: `${suggestion.title}: ${suggestion.description}`,
        triggerSource: "suggestion",
      }),
    );
  }, [
    dismissMutation,
    dispatch,
    suggestion.description,
    suggestion.id,
    suggestion.title,
  ]);

  const handleControl = useCallback(
    async (ctrl: BuddyControl) => {
      if (ctrl.action === "dismiss") {
        await handleDismiss();
        return;
      }
      if (ctrl.action === "investigate_error") {
        await handleInvestigate();
        return;
      }

      await executeBuddyAction(ctrl, dispatch, {
        triggerText: `${suggestion.title}: ${suggestion.description}`,
        triggerSource: "suggestion",
      });

      if (ctrl.action === "accept_quest" || suggestion.quest) {
        dispatch(dismissBuddySuggestion(suggestion.id));
      }
    },
    [
      dispatch,
      handleDismiss,
      handleInvestigate,
      suggestion.description,
      suggestion.id,
      suggestion.quest,
      suggestion.title,
    ],
  );

  const controls = suggestion.controls.length
    ? suggestion.controls
    : [
        {
          id: `${suggestion.id}-investigate`,
          label: "Poke trail",
          action: "investigate_error",
          style: "primary",
        },
        {
          id: `${suggestion.id}-dismiss`,
          label: "Nope goblin",
          action: "dismiss",
          style: "secondary",
        },
      ];

  return (
    <div className={styles.card} style={{ borderColor: palette.body }}>
      <div className={styles.canvasWrap}>
        <BuddyCanvas
          state={buddy.state}
          onEvent={buddy.handleCanvasEvent}
          displaySize={48}
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
        {controls.map((ctrl) => (
          <Button
            key={ctrl.id}
            size="1"
            variant={ctrl.style === "primary" ? "soft" : "ghost"}
            color={ctrl.style === "primary" ? undefined : "gray"}
            onClick={() => void handleControl(ctrl)}
          >
            {ctrl.label}
          </Button>
        ))}
      </Flex>
    </div>
  );
};

export const BuddySuggestionBar: React.FC = () => {
  const suggestions = useAppSelector(selectBuddySuggestions);
  const active = suggestions
    .filter((s) => !s.dismissed && s.suggestion_type !== "error_pattern")
    .slice(0, 1);

  if (active.length === 0) return null;

  return (
    <div className={styles.bar}>
      {active.map((s) => (
        <SuggestionCard key={s.id} suggestion={s} />
      ))}
    </div>
  );
};
