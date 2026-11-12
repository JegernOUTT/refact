import { useState, useEffect, useMemo, useCallback } from "react";
import { useDebounceCallback } from "usehooks-ts";
import { Checkboxes } from "./useCheckBoxes";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useHasCaps } from "../../hooks/useHasCaps";
import { addCheckboxValuesToInput } from "./utils";
import {
  type CommandCompletionResponse,
  commandsApi,
} from "../../services/refact/commands";
import {
  ChatContextFile,
  ChatMeta,
  UserMessage,
  UserMessageContentWithImage,
} from "../../services/refact/types";
import type { LspChatMessage } from "../../services/refact";
import {
  selectModelById,
  selectThreadModeById,
  selectThreadImagesById,
  useThreadId,
} from "../../features/Chat/Thread";
import { formatMessagesForLsp } from "../../features/Chat/Thread/utils";

const completionDebounceOptions = { leading: true, maxWait: 250 };
const previewDebounceOptions = { leading: true };

function useGetCommandCompletionQuery(
  query: string,
  cursor: number,
  skip = false,
): CommandCompletionResponse {
  const hasCaps = useHasCaps();
  const { data } = commandsApi.useGetCommandCompletionQuery(
    { query, cursor },
    { skip: !hasCaps || skip },
  );

  if (!data) {
    return {
      completions: [],
      replace: [0, 0],
      is_cmd_executable: false,
    };
  }

  return data;
}

function useCommandCompletion() {
  const [command, setCommand] = useState<{
    query: string;
    cursor: number;
  } | null>(null);

  const updateCommand = useCallback(
    (query: string, cursor: number) => setCommand({ query, cursor }),
    [setCommand],
  );
  const debounceSetCommand = useDebounceCallback(
    updateCommand,
    500,
    completionDebounceOptions,
  );

  useEffect(() => {
    return () => debounceSetCommand.cancel();
  }, [debounceSetCommand]);

  const commandCompletionResponse = useGetCommandCompletionQuery(
    command?.query ?? "",
    command?.cursor ?? 0,
    command === null,
  );

  return {
    query: command?.query ?? "",
    commands: commandCompletionResponse,
    requestCompletion: debounceSetCommand,
  };
}

function useGetCommandPreviewQuery(
  query: string,
): (ChatContextFile | string)[] {
  const hasCaps = useHasCaps();
  const hasPreviewQuery = query.trim().length > 0;
  const chatId = useThreadId();
  const attachedImages = useAppSelector((state) =>
    selectThreadImagesById(state, chatId),
  );

  const currentThreadMode = useAppSelector((state) =>
    selectThreadModeById(state, chatId),
  );
  const currentModel = useAppSelector((state) =>
    selectModelById(state, chatId),
  );

  const userMessage: UserMessage = useMemo(() => {
    if (attachedImages.length === 0) {
      return { role: "user", content: query, checkpoints: [] };
    }

    const images: UserMessageContentWithImage[] = attachedImages
      .filter((img) => typeof img.content === "string")
      .map((img) => ({
        type: "image_url" as const,
        image_url: { url: img.content as string },
      }));

    if (images.length === 0) {
      return { role: "user", content: query, checkpoints: [] };
    }

    return {
      role: "user",
      content: [...images, { type: "text" as const, text: query }],
      checkpoints: [],
    };
  }, [query, attachedImages]);

  const messagesToSend: LspChatMessage[] = useMemo(
    () => formatMessagesForLsp([userMessage]),
    [userMessage],
  );

  const metaToSend: ChatMeta = {
    chat_id: chatId,
    chat_mode: currentThreadMode ?? "AGENT",
  };

  const { data } = commandsApi.useGetCommandPreviewQuery(
    { messages: messagesToSend, meta: metaToSend, model: currentModel },
    {
      skip: !hasCaps || !hasPreviewQuery,
    },
  );

  if (!hasPreviewQuery || !data) return [];
  return data.files;
}

function useGetPreviewFiles(query: string, checkboxes: Checkboxes) {
  const queryWithCheckboxes = useMemo(
    () => addCheckboxValuesToInput(query, checkboxes),
    [checkboxes, query],
  );
  const [previewQuery, setPreviewQuery] = useState<string>(queryWithCheckboxes);

  const debounceSetPreviewQuery = useDebounceCallback(
    setPreviewQuery,
    500,
    previewDebounceOptions,
  );

  useEffect(() => {
    return () => debounceSetPreviewQuery.cancel();
  }, [debounceSetPreviewQuery]);

  useEffect(() => {
    debounceSetPreviewQuery(queryWithCheckboxes);
  }, [debounceSetPreviewQuery, queryWithCheckboxes]);

  const previewFileResponse = useGetCommandPreviewQuery(previewQuery);
  return previewFileResponse;
}

export function useCommandCompletionAndPreviewFiles(
  checkboxes: Checkboxes,
  addFilesToInput: (str: string) => string,
) {
  const { commands, requestCompletion, query } = useCommandCompletion();

  const previewFileResponse = useGetPreviewFiles(
    addFilesToInput(query),
    checkboxes,
  );

  return {
    commands,
    requestCompletion,
    previewFiles: previewFileResponse,
  };
}
