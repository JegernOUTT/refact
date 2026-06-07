import { MeteringUsd } from '../services/refact';
export declare const useTotalTokenMeteringForChat: () => {
    metering_prompt_tokens_n: number;
    metering_generated_tokens_n: number;
    metering_cache_creation_tokens_n: number;
    metering_cache_read_tokens_n: number;
} | null;
export declare const useTotalUsdForChat: () => MeteringUsd | null;
