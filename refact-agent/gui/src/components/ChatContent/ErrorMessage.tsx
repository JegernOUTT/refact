import React from "react";
import { AlertTriangle } from "lucide-react";
import { Button, Icon } from "../ui";
import type { ErrorMessage } from "../../services/refact/types";
import { collectErrorTexts } from "../../services/refact/recoveryPresentation";
import styles from "./ErrorMessage.module.css";

export type ErrorMessageCardProps = {
  errors: ErrorMessage[];
  onRetryGeneration?: () => void;
};

export const ErrorMessageCard: React.FC<ErrorMessageCardProps> = ({
  errors,
  onRetryGeneration,
}) => {
  const texts = collectErrorTexts(errors, []);
  if (texts.length === 0) return null;
  return (
    <section
      className={styles.errors}
      data-testid="error-message-card"
      aria-label="Request error"
    >
      <div className={styles.heading}>
        <Icon icon={AlertTriangle} size="sm" />
        <strong>Request failed</strong>
      </div>
      <div role="alert" className={styles.errors}>
        {texts.map((text) => (
          <div className={styles.message} key={text}>
            {text}
          </div>
        ))}
      </div>
      {onRetryGeneration && (
        <Button size="sm" variant="soft" onClick={onRetryGeneration}>
          Retry
        </Button>
      )}
    </section>
  );
};
