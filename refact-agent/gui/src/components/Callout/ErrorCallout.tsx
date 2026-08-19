import React from "react";
import { useAppSelector } from "../../hooks/useAppSelector";
import { getIsAuthError } from "../../features/Errors/errorsSlice";
import { ErrorCalloutView, type ErrorCalloutViewProps } from "./Callout";

export type ErrorCalloutProps = Omit<ErrorCalloutViewProps, "isAuthError">;

/**
 * Store-connected error callout: derives the auth-error flag from Redux so
 * the presentational Callout module stays store-free (audit L-11).
 */
export const ErrorCallout: React.FC<ErrorCalloutProps> = (props) => {
  const isAuthError = useAppSelector(getIsAuthError);
  return <ErrorCalloutView isAuthError={isAuthError} {...props} />;
};
