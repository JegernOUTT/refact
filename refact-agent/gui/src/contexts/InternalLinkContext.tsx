import React, { createContext, useContext } from "react";

export type InternalLinkHandler = (url: string) => boolean;

interface InternalLinkContextValue {
  handleInternalLink: InternalLinkHandler;
}

const InternalLinkContext = createContext<InternalLinkContextValue | null>(null);

export const useInternalLinkHandler = () => {
  return useContext(InternalLinkContext);
};

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
    [onInternalLink]
  );

  return (
    <InternalLinkContext.Provider value={value}>
      {children}
    </InternalLinkContext.Provider>
  );
};

export const parseRefactLink = (url: string): { type: string; id: string } | null => {
  if (!url.startsWith("refact://")) return null;

  const withoutProtocol = url.substring("refact://".length);
  const [type, ...rest] = withoutProtocol.split("/");
  const id = rest.join("/");

  return { type, id };
};
