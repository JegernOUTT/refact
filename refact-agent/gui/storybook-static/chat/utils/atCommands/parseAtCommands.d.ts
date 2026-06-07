import type { AtCommandToken, LineRange, ParsedLine } from "./types";
export declare function parseLineRange(arg: string): {
    path: string;
    lineRange?: LineRange;
};
export declare function formatLineRange(lineRange: LineRange): string;
export declare function parseLine(line: string): ParsedLine;
export declare function parseLines(text: string): ParsedLine[];
export declare function hasAtCommands(parsedLines: ParsedLine[]): boolean;
export declare function getAtCommandTokens(parsedLines: ParsedLine[]): AtCommandToken[];
