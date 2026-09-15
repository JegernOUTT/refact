import React, { useCallback } from "react";
import { Flex, Text } from "@radix-ui/themes";
import { Callout } from "../../components/Callout";
import { Button } from "../../components/ui";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  clearError,
  getCapsProvidersErrorMessage,
  getIsAuthError,
} from "./errorsSlice";
import styles from "./CapsProvidersErrorBanner.module.css";

export type CapsProvidersErrorBannerProps = {
  onRetry: () => void;
  isRetrying: boolean;
};

export const CapsProvidersErrorBanner: React.FC<
  CapsProvidersErrorBannerProps
> = ({ onRetry, isRetrying }) => {
  const dispatch = useAppDispatch();
  const message = useAppSelector(getCapsProvidersErrorMessage);
  const isAuthError = useAppSelector(getIsAuthError);

  const onDismiss = useCallback(() => {
    dispatch(clearError());
  }, [dispatch]);

  if (!message) return null;

  return (
    <div className={styles.region}>
      <Callout
        type="error"
        role="alert"
        preventClose
        className={styles.banner}
        data-element="caps-providers-error-banner"
      >
        <Flex direction="column" align="start" gap="2">
          <Text as="span">{message}</Text>
          <Flex gap="2">
            {!isAuthError && (
              <Button
                variant="soft"
                size="sm"
                onClick={onRetry}
                disabled={isRetrying}
                loading={isRetrying}
              >
                Retry
              </Button>
            )}
            <Button variant="ghost" size="sm" onClick={onDismiss}>
              Dismiss
            </Button>
          </Flex>
        </Flex>
      </Callout>
    </div>
  );
};
