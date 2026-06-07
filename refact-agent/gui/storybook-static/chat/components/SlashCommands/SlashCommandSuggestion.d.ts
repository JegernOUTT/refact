import React from "react";
import type { CompletionDetail } from "../../services/refact/commands";
type SlashCommandSuggestionProps = {
    name: string;
    detail?: CompletionDetail;
};
export declare const SlashCommandSuggestion: React.FC<SlashCommandSuggestionProps>;
export {};
