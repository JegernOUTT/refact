import React from "react";
import classNames from "classnames";
import { TriangleAlert } from "lucide-react";
import { Icon } from "../Icon";
import styles from "./ErrorState.module.css";

export type ErrorStateVariant = "compact" | "full";

export interface ErrorStateProps
  extends Omit<React.ComponentPropsWithoutRef<"section">, "title"> {
  title: React.ReactNode;
  description?: React.ReactNode;
  error?: unknown;
  retry?: React.ReactNode;
  variant?: ErrorStateVariant;
}

function errorMessage(error: unknown): React.ReactNode {
  if (!error) return null;
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Something went wrong.";
}

export function ErrorState({
  title,
  description,
  error,
  retry,
  variant = "compact",
  className,
  ...props
}: ErrorStateProps) {
  const details = description ?? errorMessage(error);

  return (
    <section
      className={classNames(styles.errorState, styles[variant], className)}
      role="alert"
      {...props}
    >
      <div className={styles.iconWrap}>
        <Icon icon={TriangleAlert} size={variant === "full" ? "lg" : "md"} tone="danger" />
      </div>
      <div className={styles.copy}>
        <h3 className={styles.title}>{title}</h3>
        {details ? <p className={styles.description}>{details}</p> : null}
      </div>
      {retry ? <div className={styles.retry}>{retry}</div> : null}
    </section>
  );
}
