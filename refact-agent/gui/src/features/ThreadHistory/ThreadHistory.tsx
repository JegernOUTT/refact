import { FC, useCallback, useMemo } from "react";
import { Config } from "../Config/configSlice";
import { Button, Flex, Spinner, Text } from "@radix-ui/themes";
import { ArrowLeftIcon } from "@radix-ui/react-icons";
import { ChatRawJSON } from "../../components/ChatRawJSON";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectThreadById } from "../Chat/Thread/selectors";
import {
  useGetTrajectoryQuery,
  trajectoryDataToChatThread,
} from "../../services/refact";
import { copyChatHistoryToClipboard } from "../../utils/copyChatHistoryToClipboard";
import { clearError, getErrorMessage, setError } from "../Errors/errorsSlice";
import {
  clearInformation,
  getInformationMessage,
  setInformation,
} from "../Errors/informationSlice";
import {
  ErrorCallout,
  InformationCallout,
} from "../../components/Callout/Callout";
import styles from "./ThreadHistory.module.css";

type ThreadHistoryProps = {
  onCloseThreadHistory: () => void;
  backFromThreadHistory: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  chatId: string;
};

export const ThreadHistory: FC<ThreadHistoryProps> = ({
  onCloseThreadHistory,
  backFromThreadHistory,
  host,
  tabbed,
  chatId,
}) => {
  const dispatch = useAppDispatch();

  const activeThread = useAppSelector((state) =>
    selectThreadById(state, chatId),
  );

  const {
    data: trajectoryData,
    isLoading,
    error: fetchError,
  } = useGetTrajectoryQuery(chatId, {
    skip: Boolean(activeThread && activeThread.messages.length > 0),
  });

  const historyThreadToPass = useMemo(() => {
    if (activeThread && activeThread.messages.length > 0) {
      return {
        ...activeThread,
        model: activeThread.model || "gpt-4o-mini",
      };
    }
    if (trajectoryData) {
      const thread = trajectoryDataToChatThread(trajectoryData);
      return {
        ...thread,
        model: thread.model || "gpt-4o-mini",
      };
    }
    return null;
  }, [activeThread, trajectoryData]);

  const error = useAppSelector(getErrorMessage);
  const information = useAppSelector(getInformationMessage);

  const onClearError = useCallback(() => dispatch(clearError()), [dispatch]);
  const onClearInformation = useCallback(
    () => dispatch(clearInformation()),
    [dispatch],
  );

  const handleCopyToClipboardJSON = useCallback(() => {
    if (!historyThreadToPass) {
      dispatch(setError("No history thread found"));
      return;
    }

    void copyChatHistoryToClipboard(historyThreadToPass).then(() => {
      dispatch(setInformation("Chat history copied to clipboard"));
    });
  }, [dispatch, historyThreadToPass]);

  const handleBackFromThreadHistory = useCallback(
    (customBackFunction: () => void) => {
      if (information) {
        onClearInformation();
      }
      if (error) {
        onClearError();
      }
      customBackFunction();
    },
    [information, error, onClearError, onClearInformation],
  );

  return (
    <>
      {host === "vscode" && !tabbed ? (
        <Flex gap="2" pb="3">
          <Button
            variant="surface"
            onClick={() => handleBackFromThreadHistory(backFromThreadHistory)}
          >
            <ArrowLeftIcon width="16" height="16" />
            Back
          </Button>
        </Flex>
      ) : (
        <Button
          mr="auto"
          variant="outline"
          onClick={() => handleBackFromThreadHistory(onCloseThreadHistory)}
          mb="4"
        >
          Back
        </Button>
      )}
      {isLoading && (
        <Flex align="center" justify="center" py="6" gap="2">
          <Spinner size="2" />
          <Text size="2" color="gray">
            Loading thread history...
          </Text>
        </Flex>
      )}
      {fetchError && !historyThreadToPass && (
        <Text size="2" color="red">
          Failed to load thread history
        </Text>
      )}
      {historyThreadToPass && (
        <ChatRawJSON
          thread={historyThreadToPass}
          copyHandler={handleCopyToClipboardJSON}
        />
      )}
      {information && (
        <InformationCallout
          className={styles.calloutContainer}
          onClick={onClearInformation}
          timeout={3000}
        >
          {information}
        </InformationCallout>
      )}
      {error && (
        <ErrorCallout
          className={styles.calloutContainer}
          onClick={onClearError}
          timeout={3000}
        >
          {error}
        </ErrorCallout>
      )}
    </>
  );
};
