import type { Usage, PromptTokenDetails, CompletionTokenDetails } from "../services/refact";
/**
 * Calculates the sum of token values from specified keys in a usage object
 *
 * @param options - Configuration object
 * @param options.keys - Array of keys to extract values from the usage object
 * @param options.usage - The usage object to extract values from
 * @returns The sum of all numeric values from the specified keys, or 0 if usage is undefined
 *
 * @example
 * ```typescript
 * const tokens = calculateUsageInputTokens({
 *   keys: ['prompt_tokens', 'completion_tokens'],
 *   usage: { prompt_tokens: 100, completion_tokens: 50, total_tokens: 150 }
 * }); // returns 150
 * ```
 */
export declare const calculateUsageInputTokens: ({ keys, usage, }: {
    keys: (keyof Usage)[];
    usage?: Usage | null;
}) => number;
export declare function getCacheCreationTokens(usage?: Usage | null): number;
export declare function getCacheReadTokens(usage?: Usage | null): number;
/**
 * Safely sums two numeric values, treating null or undefined values as zero
 *
 * @param a - First number (or null/undefined)
 * @param b - Second number (or null/undefined)
 * @returns The sum of both values, substituting 0 for null/undefined values
 *
 * @example
 * ```typescript
 * sumValues(5, 10); // returns 15
 * sumValues(null, 5); // returns 5
 * sumValues(undefined, undefined); // returns 0
 * ```
 */
export declare const sumValues: (a?: number | null, b?: number | null) => number;
/**
 * Merges two completion token details objects, combining their numeric values
 *
 * @param a - First completion token details object (or null/undefined)
 * @param b - Second completion token details object (or null/undefined)
 * @returns A new merged completion token details object, or null if both inputs are nullish
 *
 * @example
 * ```typescript
 * const details1 = { accepted_prediction_tokens: 10, audio_tokens: 5, reasoning_tokens: 20, rejected_prediction_tokens: 2 };
 * const details2 = { accepted_prediction_tokens: 15, audio_tokens: 0, reasoning_tokens: 10, rejected_prediction_tokens: 3 };
 * mergeCompletionTokensDetails(details1, details2);
 * // returns { accepted_prediction_tokens: 25, audio_tokens: 5, reasoning_tokens: 30, rejected_prediction_tokens: 5 }
 * ```
 */
export declare const mergeCompletionTokensDetails: (a?: CompletionTokenDetails | null, b?: CompletionTokenDetails | null) => CompletionTokenDetails | null;
/**
 * Merges two prompt token details objects, combining their numeric values
 *
 * @param a - First prompt token details object (or null/undefined)
 * @param b - Second prompt token details object (or null/undefined)
 * @returns A new merged prompt token details object, or null if both inputs are nullish
 *
 * @example
 * ```typescript
 * const details1 = { audio_tokens: 5, cached_tokens: 100 };
 * const details2 = { audio_tokens: 10, cached_tokens: 200 };
 * mergePromptTokensDetails(details1, details2);
 * // returns { audio_tokens: 15, cached_tokens: 300 }
 * ```
 */
export declare const mergePromptTokensDetails: (a?: PromptTokenDetails | null, b?: PromptTokenDetails | null) => PromptTokenDetails | null;
/**
 * Combines multiple usage records into a single aggregated usage record
 *
 * This function takes an array of Usage objects and merges them into a single
 * Usage object by summing all numerical values and properly combining nested
 * token detail objects. It handles undefined values and ensures proper type safety.
 *
 * @param usages - Array of Usage objects to merge (may contain undefined values)
 * @returns A new merged Usage object containing the sum of all input values, or undefined if the input array is empty or contains only undefined values
 *
 * @example
 * ```typescript
 * const usage1 = { completion_tokens: 30, prompt_tokens: 100, total_tokens: 130 };
 * const usage2 = { completion_tokens: 20, prompt_tokens: 80, total_tokens: 100 };
 * mergeUsages([usage1, usage2]);
 * // returns { completion_tokens: 50, prompt_tokens: 180, total_tokens: 230, ... }
 * ```
 */
export declare function mergeUsages(usages: (Usage | undefined | null)[]): Usage | undefined;
