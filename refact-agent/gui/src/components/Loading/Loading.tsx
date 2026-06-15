import React from "react";
import { LoadingState } from "../ui";

export const Loading: React.FC = () => {
  return <LoadingState kind="skeleton" label={null} variant="compact" />;
};

Loading.displayName = "Loading";
