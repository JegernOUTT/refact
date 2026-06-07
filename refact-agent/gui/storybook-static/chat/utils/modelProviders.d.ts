export declare function isLegacyRefactModel(modelName: string): boolean;
/**
 * Extract provider name from model name (e.g., "openai/gpt-4o" -> "openai")
 */
export declare function extractProvider(modelName: string): string;
/**
 * Get display name for provider
 */
export declare function getProviderDisplayName(provider: string): string;
