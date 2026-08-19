import React, { useEffect, useState } from "react";
import { Flex, Text } from "@radix-ui/themes";
import { AlertTriangle, Info } from "lucide-react";
import { useTimeout } from "usehooks-ts";
import classNames from "classnames";
import { Icon, Surface } from "../ui";
import styles from "./Callout.module.css";

export type CalloutProps = Omit<
  React.ComponentPropsWithoutRef<"div">,
  "onClick" | "color"
> & {
  type: "info" | "error" | "warning";
  onClick?: () => void;
  timeout?: number | null;
  preventClose?: boolean;
};

export const Callout: React.FC<CalloutProps> = ({
  children,
  type = "info",
  timeout = null,
  onClick = () => void 0,
  preventClose = false,
  className,
  ...props
}) => {
  const [isOpened, setIsOpened] = useState(false);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setIsOpened(true);
    }, 150);

    return () => {
      window.clearTimeout(timeoutId);
    };
  }, []);

  const handleRetryClick = () => {
    if (preventClose) {
      onClick();
      return;
    }
    setIsOpened(false);
    const timeoutId = window.setTimeout(() => {
      onClick();
      window.clearTimeout(timeoutId);
    }, 300);
  };

  useTimeout(handleRetryClick, timeout);

  return (
    <Surface
      as="div"
      onClick={handleRetryClick}
      {...props}
      className={classNames(
        styles.callout_box,
        styles[`callout_box_${type}`],
        {
          [styles.callout_box_opened]: isOpened,
        },
        className,
      )}
      radius="card"
      variant="surface-1"
    >
      <Flex direction="row" align="center" gap="4">
        <Icon
          icon={type === "info" ? Info : AlertTriangle}
          tone={
            type === "error"
              ? "danger"
              : type === "warning"
                ? "warning"
                : "accent"
          }
        />
        <Flex direction="column" align="start" gap="1">
          <Text as="div" className={styles.callout_text} wrap="wrap">
            {children}
          </Text>
        </Flex>
      </Flex>
    </Surface>
  );
};

export type ErrorCalloutViewProps = Omit<CalloutProps, "type"> & {
  /**
   * Presentational flag; the store-connected wrapper in ErrorCallout.tsx
   * derives it from auth state so this file stays store-free.
   */
  isAuthError?: boolean;
  preventRetry?: boolean;
};

export const ErrorCalloutView: React.FC<ErrorCalloutViewProps> = ({
  timeout = null,
  onClick,
  children,
  preventRetry,
  preventClose = false,
  className,
  isAuthError = false,
  ...props
}) => {
  return (
    <Callout
      type="error"
      onClick={onClick}
      timeout={timeout}
      preventClose={preventClose || isAuthError}
      className={classNames(styles.callout_box_inner, className)}
      {...props}
    >
      Error: {children}
      {!isAuthError && (
        <Text size="1" as="span" className={styles.retryText}>
          {preventRetry ? "Click to close" : "Click to retry"}
        </Text>
      )}
      {isAuthError && (
        <Flex as="span" gap="2" mt="3">
          Check your provider configuration, API key, and network access.
        </Flex>
      )}
    </Callout>
  );
};

export const InformationCallout: React.FC<Omit<CalloutProps, "type">> = ({
  timeout = null,
  onClick,
  children,
  ...props
}) => {
  return (
    <Callout type="info" onClick={onClick} timeout={timeout} {...props}>
      Info: {children}
    </Callout>
  );
};

export type DiffWarningCalloutProps = Omit<CalloutProps, "type"> & {
  message?: string | string[] | null;
};

export const DiffWarningCallout: React.FC<DiffWarningCalloutProps> = ({
  timeout = null,
  onClick,
  message = null,
  children,
  ...props
}) => {
  const warningMessages = !message
    ? ["Some error occurred"]
    : Array.isArray(message)
      ? message
      : [message];

  return (
    <Callout type="warning" onClick={onClick} timeout={timeout} {...props}>
      <Flex direction="column" gap="1">
        {warningMessages.map((msg, i) => (
          <span key={`${msg}-${i}`}>{i === 0 ? `Warning: ${msg}` : msg}</span>
        ))}
        {children}
      </Flex>
    </Callout>
  );
};

type CalloutFromTopProps = React.ComponentPropsWithoutRef<"div"> & {
  children?: React.ReactNode;
};

export function CalloutFromTop(props: CalloutFromTopProps) {
  return (
    <Surface
      {...props}
      className={styles.callout_from_top}
      radius="card"
      variant="surface-1"
    >
      <Flex direction="row" align="center" gap="4" position="relative">
        <Icon icon={Info} tone="accent" />
        {props.children}
      </Flex>
    </Surface>
  );
}
