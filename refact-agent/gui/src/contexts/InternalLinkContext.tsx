import React, { createContext } from "react";

export type InternalLinkOrigin = {
  chatId?: string;
  taskId?: string;
  cardId?: string;
};

export type InternalLinkHandler = (
  url: string,
  origin?: InternalLinkOrigin,
) => boolean | Promise<boolean>;

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
