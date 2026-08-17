import React from "react";
import { v4 as uuidv4 } from "uuid";

import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  createChatWithId,
  selectModelById,
  selectPrivacyStepContextById,
  selectThreadById,
  switchToThread,
} from "../Chat/Thread";
import { push } from "../Pages/pagesSlice";
import { selectApiKey, selectConfig } from "../Config/configSlice";
import { branchFromChat } from "../../services/refact/chatCommands";
import type { ToolResult } from "../../services/refact/types";
import {
  blockedPrivacyFilesFromInspection,
  extractPrivacyFiles,
  extractPrivacyShellMetadata,
  isPrivacyRefusalContent,
  privacyDestinationForModel,
  useInspectPrivacyQuery,
} from "../../services/refact/privacy";
import { BlockCard } from "./BlockCard";
import { WithheldOutputCard } from "./WithheldOutputCard";

type PrivacyToolResultProps = {
  threadId: string;
  toolCallId: string;
  result: ToolResult;
};

export const PrivacyToolResult: React.FC<PrivacyToolResultProps> = ({
  threadId,
  toolCallId,
  result,
}) => {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const model = useAppSelector((state) => selectModelById(state, threadId));
  const thread = useAppSelector((state) => selectThreadById(state, threadId));
  const stepContext = useAppSelector((state) =>
    selectPrivacyStepContextById(state, threadId, toolCallId),
  );
  const files = extractPrivacyFiles(result.extra);
  const shell = extractPrivacyShellMetadata(result.extra);
  const destination = privacyDestinationForModel(model);
  const refusal = isPrivacyRefusalContent(result.content);
  const inspection = useInspectPrivacyQuery(
    { chat_id: threadId, destination, records: files },
    { skip: shell !== null || !refusal || !model },
  );
  const blockedFiles = blockedPrivacyFilesFromInspection(
    files,
    inspection.data,
  );

  const handleSwitchModel = React.useCallback(() => {
    dispatch(push({ name: "default models" }));
  }, [dispatch]);

  const handleBranch = React.useCallback(() => {
    if (!stepContext.branchMessageId) return;
    const newChatId = uuidv4();
    dispatch(
      createChatWithId({
        id: newChatId,
        title: `[privacy branch] ${thread?.title ?? "Chat"}`,
        parentId: threadId,
        linkType: "branch",
        worktree: thread?.worktree,
      }),
    );
    dispatch(switchToThread({ id: newChatId }));
    dispatch(push({ name: "chat" }));
    void branchFromChat(
      newChatId,
      threadId,
      stepContext.branchMessageId,
      config,
      apiKey ?? undefined,
    ).catch(() => undefined);
  }, [
    apiKey,
    stepContext.branchMessageId,
    config,
    dispatch,
    thread?.title,
    thread?.worktree,
    threadId,
  ]);

  if (shell) {
    const exitCode =
      typeof result.extra?.exec === "object" &&
      result.extra.exec !== null &&
      "exit_code" in result.extra.exec &&
      typeof result.extra.exec.exit_code === "number"
        ? result.extra.exec.exit_code
        : null;
    return (
      <WithheldOutputCard
        exitCode={exitCode}
        files={files}
        localOnlyOutput={shell.local_only_output}
      />
    );
  }

  const awaitingInspection = !inspection.data && !inspection.isError;
  const inspectionAllowsSend = inspection.data?.sendable === true;
  if (
    !refusal ||
    awaitingInspection ||
    inspectionAllowsSend ||
    blockedFiles.length === 0
  ) {
    return null;
  }

  return (
    <BlockCard
      model={model}
      step={stepContext.step}
      blockedFiles={blockedFiles}
      onSwitchModel={handleSwitchModel}
      onBranchCleanChat={handleBranch}
    />
  );
};
