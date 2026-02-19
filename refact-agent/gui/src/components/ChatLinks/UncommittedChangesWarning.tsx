import React from "react";
import { useAppSelector, useGetLinksFromLsp } from "../../hooks";
import { Markdown } from "../Markdown";
import { Flex, Separator } from "@radix-ui/themes";
import {
  selectIsStreaming,
  selectIsWaiting,
  selectMessages,
  selectThreadMode,
} from "../../features/Chat";
import { getErrorMessage } from "../../features/Errors/errorsSlice";
import { getInformationMessage } from "../../features/Errors/informationSlice";
import { useGetChatModesQuery } from "../../services/refact/chatModes";

export const UncommittedChangesWarning: React.FC = () => {
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const linksRequest = useGetLinksFromLsp();
  const error = useAppSelector(getErrorMessage);
  const information = useAppSelector(getInformationMessage);
  const currentMode = useAppSelector(selectThreadMode);
  const messages = useAppSelector(selectMessages);
  const modesQuery = useGetChatModesQuery(undefined);

  const modeHasEditing = React.useMemo(() => {
    if (!modesQuery.data?.modes) return false;
    const modeInfo = modesQuery.data.modes.find((m) => m.id === currentMode);
    if (!modeInfo) return currentMode === "agent";
    return modeInfo.ui.tags.includes("editing");
  }, [modesQuery.data?.modes, currentMode]);

  const hasCallout = React.useMemo(() => {
    return !!error || !!information;
  }, [error, information]);

  if (
    !modeHasEditing ||
    messages.length !== 0 ||
    hasCallout ||
    isStreaming ||
    isWaiting ||
    linksRequest.isFetching ||
    linksRequest.isLoading ||
    !linksRequest.data?.uncommited_changes_warning
  ) {
    return false;
  }

  return (
    <Flex py="4" gap="4" direction="column" justify="between">
      <Separator size="4" />
      <Markdown>{linksRequest.data.uncommited_changes_warning}</Markdown>
    </Flex>
  );
};
