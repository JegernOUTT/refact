import React from "react";
import { Callout, Button, Flex } from "@radix-ui/themes";
import { ExclamationTriangleIcon } from "@radix-ui/react-icons";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import {
  selectConnectionProblem,
  selectBackendStatus,
  selectCurrentChatSseStatus,
} from "../../features/Connection";
import { requestSseRefresh } from "../../features/Chat/Thread/actions";
import { selectCurrentThreadId } from "../../features/Chat/Thread/selectors";
import styles from "./ConnectionStatus.module.css";

export const ConnectionBanner: React.FC = () => {
  const dispatch = useAppDispatch();
  const problem = useAppSelector(selectConnectionProblem);
  const backendStatus = useAppSelector(selectBackendStatus);
  const sseStatus = useAppSelector(selectCurrentChatSseStatus);
  const currentThreadId = useAppSelector(selectCurrentThreadId);

  const handleRetry = () => {
    if (currentThreadId) {
      dispatch(requestSseRefresh({ chatId: currentThreadId }));
    }
  };

  if (!problem) return null;
  if (backendStatus === "unknown") return null;

  const isReconnecting = sseStatus === "connecting";

  return (
    <Callout.Root color="orange" size="1" className={styles.banner}>
      <Callout.Icon>
        <ExclamationTriangleIcon />
      </Callout.Icon>
      <Flex justify="between" align="center" width="100%" gap="2">
        <Callout.Text>
          {problem}
          {isReconnecting && "..."}
        </Callout.Text>
        {!isReconnecting && (
          <Button size="1" variant="soft" onClick={handleRetry}>
            Retry
          </Button>
        )}
      </Flex>
    </Callout.Root>
  );
};

export default ConnectionBanner;
