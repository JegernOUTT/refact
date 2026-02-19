import React from "react";
import { TextArea, type TextAreaProps } from "../TextArea/TextArea";

type TextAreaWithChipsProps = TextAreaProps & {
  host: string;
  onOpenFile?: (file: { file_path: string; line?: number }) => Promise<void>;
};

export const TextAreaWithChips = React.forwardRef<
  HTMLTextAreaElement,
  TextAreaWithChipsProps
>(({ host: _host, onOpenFile: _onOpenFile, ...props }, ref) => {
  return <TextArea {...props} ref={ref} />;
});

TextAreaWithChips.displayName = "TextAreaWithChips";
