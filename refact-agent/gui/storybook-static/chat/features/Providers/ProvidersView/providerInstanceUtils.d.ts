import type { ProviderListItem } from "../../../services/refact";
export type ProviderBaseOption = {
    id: string;
    label: string;
};
export declare function providerBaseLabel(baseProvider: string): string;
export declare function providerInstanceDisplayName(baseProvider: string, instanceId: string): string;
export declare function nextInstanceId(baseProvider: string, providerNames: string[]): string;
export declare function providerBaseOptions(providers: ProviderListItem[]): ProviderBaseOption[];
export declare function validateProviderInstanceId(instanceId: string, providerNames: string[]): "Instance id is required." | "Instance id must be 64 characters or fewer." | "This instance id is reserved." | "Instance id must start with an ASCII letter or digit." | "Instance id must not contain path characters." | "Use ASCII letters, numbers, underscores, and hyphens only." | "A provider with this id already exists." | null;
