import { useContext } from "react";
import { InternalLinkContext } from "./InternalLinkContext";

export const useInternalLinkHandler = () => {
  const context = useContext(InternalLinkContext);
  return context;
};

export const parseRefactLink = (
  url: string,
): { type: string; id: string } | null => {
  if (!url.startsWith("refact://")) return null;

  const withoutProtocol = url.substring("refact://".length);
  const [type, ...rest] = withoutProtocol.split("/");
  const id = rest.join("/");

  return { type, id };
};
