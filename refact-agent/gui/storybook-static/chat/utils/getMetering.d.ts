import { ChatMessages, MeteringUsd } from "../services/refact/types";
export declare function getTotalTokenMeteringForMessages(messages: ChatMessages): {
    metering_prompt_tokens_n: number;
    metering_generated_tokens_n: number;
    metering_cache_creation_tokens_n: number;
    metering_cache_read_tokens_n: number;
} | null;
export declare function getTotalUsdMeteringForMessages(messages: ChatMessages): MeteringUsd | null;
export declare function formatUsd(value: number | undefined): string;
