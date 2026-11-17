import React from "react";
import { Flex, Text } from "../../components/ui";
import { reportBuddyFrontendError } from "./reportBuddyFrontendError";
import { ChatCrashReportPanel } from "./ChatCrashReportPanel";
import styles from "./BuddyErrorBoundary.module.css";

type Props = {
  children?: React.ReactNode;
  showThreadReportPanel?: boolean;
};

type State = {
  failed: boolean;
  errorDetails: string | null;
};

export class BuddyErrorBoundary extends React.Component<Props, State> {
  override state: State = {
    failed: false,
    errorDetails: null,
  };

  static getDerivedStateFromError(error: Error): State {
    return {
      failed: true,
      errorDetails: error.stack ?? error.message,
    };
  }

  override componentDidCatch(error: Error, errorInfo: React.ErrorInfo): void {
    const details = errorInfo.componentStack
      ? `${error.stack ?? error.message}\n\nComponent stack:\n${
          errorInfo.componentStack
        }`
      : error.stack ?? error.message;

    this.setState({ errorDetails: details });

    void reportBuddyFrontendError({
      source: "react_error_boundary",
      error: details,
      sourceFile: "frontend/react_error_boundary",
      toolName: "react_error_boundary",
    });
  }

  override render(): React.ReactNode {
    if (this.state.failed) {
      if (this.props.showThreadReportPanel === true) {
        return <ChatCrashReportPanel errorDetails={this.state.errorDetails} />;
      }

      return (
        <Flex align="center" justify="center" className={styles.root}>
          <div className={styles.card} role="alert">
            <Text size="3" weight="bold">
              The app hit a frontend error.
            </Text>
            <Text size="2" color="gray">
              Your companion recorded it for investigation. Reload the view if
              it stays blank.
            </Text>
          </div>
        </Flex>
      );
    }

    return this.props.children;
  }
}
