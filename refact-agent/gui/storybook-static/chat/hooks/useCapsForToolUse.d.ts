import { CapsResponse } from '../events';
export declare const PAID_AGENT_LIST: string[];
export declare const UNLIMITED_PRO_MODELS_LIST: string[];
export declare function useCapsForToolUse(): {
    usableModels: string[];
    usableModelsForPlan: {
        value: string;
        disabled: boolean;
        textValue: string;
    }[];
    currentModel: string;
    setCapModel: (value: string) => void;
    isMultimodalitySupportedForCurrentModel: boolean;
    loading: boolean;
    uninitialized: boolean;
    data: CapsResponse | undefined;
    modelsSupportingTools: string[];
    modelsSupportingAgent: string[];
    modeRequiresTools: boolean;
    modeRequiresAgent: boolean;
};
