import React, { useMemo } from "react";

import { Flex, Text } from "@radix-ui/themes";
import { MagicWandIcon } from "@radix-ui/react-icons";

import { useAppSelector } from "../../hooks";
import {
  selectActiveSpeech,
  selectBuddyActivities,
  selectBuddyState,
  selectBuddySuggestions,
  selectNowPlaying,
  selectPulse,
  selectUnreadOpportunities,
} from "../../features/Buddy/buddySlice";

const BUDDY_HELLOS = [
  "Buddy is here, ears up, ready to chase down the next bug.",
  "Buddy says hi. Drop a repo mystery here and we'll sniff it out together.",
  "Buddy is warming up the tiny debugger lantern.",
  "Buddy is watching the workspace for interesting clues.",
  "Buddy has snacks, curiosity, and a suspiciously large pile of context.",
];

const pickHello = () =>
  BUDDY_HELLOS[Math.floor(Math.random() * BUDDY_HELLOS.length)];

function firstItem<T>(items: readonly T[]): T | undefined {
  return items.length > 0 ? items[0] : undefined;
}

const BuddyInsight: React.FC = () => {
  const activeSpeech = useAppSelector(selectActiveSpeech);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const unreadOpportunities = useAppSelector(selectUnreadOpportunities);
  const suggestions = useAppSelector(selectBuddySuggestions);
  const activities = useAppSelector(selectBuddyActivities);
  const buddyState = useAppSelector(selectBuddyState);
  const pulse = useAppSelector(selectPulse);
  const fallbackHello = useMemo(pickHello, []);
  const activeSpeechText = activeSpeech?.text;

  const insight = useMemo(() => {
    const opportunity = firstItem(unreadOpportunities);
    const suggestion = suggestions.find((item) => !item.dismissed);
    const activity = firstItem(activities);

    if (activeSpeechText) return activeSpeechText;
    if (opportunity) return opportunity.summary;
    if (nowPlaying?.speech_text) return nowPlaying.speech_text;
    if (nowPlaying?.description) return nowPlaying.description;
    if (suggestion !== undefined) {
      return `${suggestion.title}: ${suggestion.description}`;
    }
    if (activity !== undefined)
      return `${activity.title}: ${activity.description}`;
    if (pulse?.humor) return pulse.humor;
    if (buddyState?.semantic.headline) return buddyState.semantic.headline;
    return fallbackHello;
  }, [
    activeSpeechText,
    activities,
    buddyState?.semantic.headline,
    fallbackHello,
    nowPlaying?.description,
    nowPlaying?.speech_text,
    pulse?.humor,
    suggestions,
    unreadOpportunities,
  ]);

  return (
    <Text as="div">
      <Flex as="span" align="center" gap="1" display="inline-flex">
        <MagicWandIcon />
        <b>Buddy</b>:
      </Flex>{" "}
      {insight}
    </Text>
  );
};

export const PlaceHolderText: React.FC = () => {
  return (
    <Flex direction="column" gap="4">
      <Text>Welcome to Refact chat.</Text>
      <BuddyInsight />
    </Flex>
  );
};
