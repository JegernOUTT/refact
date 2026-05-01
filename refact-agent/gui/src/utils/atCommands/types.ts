export type AtCommandType =
  | "file"
  | "web"
  | "tree"
  | "search"
  | "definition"
  | "knowledge-load"
  | "references"
  | "help";

export type LineRange = {
  line1: number;
  line2?: number;
  kind: "single" | "range" | "from-start" | "to-end";
};

export type AtCommandToken = {
  kind: "at";
  type: AtCommandType;
  raw: string;
  command: string;
  arg?: string;
  lineRange?: LineRange;
  startIndex: number;
  endIndex: number;
};

export type TextToken = {
  kind: "text";
  text: string;
  startIndex: number;
  endIndex: number;
};

export type Token = AtCommandToken | TextToken;

export type ParsedLine = {
  tokens: Token[];
  originalText: string;
};

export type ChipDisplayInfo = {
  type: AtCommandType;
  label: string;
  fullPath?: string;
  lineRange?: string;
  url?: string;
  disabled: boolean;
  onClick?: () => void;
};
