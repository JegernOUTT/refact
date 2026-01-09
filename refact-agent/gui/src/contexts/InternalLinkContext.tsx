import React, { createContext } from "react";

export type InternalLinkHandler = (url: string) => boolean;

export interface InternalLinkContextValue {
  handleInternalLink: InternalLinkHandler;
}

export const InternalLinkContext =
  createContext<InternalLinkContextValue | null>(null);

interface InternalLinkProviderProps {
  onInternalLink: InternalLinkHandler;
  children: React.ReactNode;
}

export const InternalLinkProvider: React.FC<InternalLinkProviderProps> = ({
  onInternalLink,
  children,
}) => {
  const value = React.useMemo(
    () => ({ handleInternalLink: onInternalLink }),
    [onInternalLink],
  );

  return (
    <InternalLinkContext.Provider value={value}>
      {children}
    </InternalLinkContext.Provider>
  );
};
