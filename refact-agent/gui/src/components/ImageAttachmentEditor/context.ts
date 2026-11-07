import { createContext } from "react";

export type ImageEditRequest = {
  file: File;
  accept: (file: File) => void;
};

export const ImageAttachmentEditorContext = createContext<
  ((requests: ImageEditRequest[]) => void) | null
>(null);
