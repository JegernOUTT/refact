import React from "react";
export type SamplingValues = {
    max_new_tokens?: number;
    top_p?: number;
    boost_reasoning?: boolean;
    reasoning_effort?: string;
    thinking_budget?: number;
};
type ModelSamplingParamsProps = {
    model: string | undefined;
    values: SamplingValues;
    onChange: <K extends keyof SamplingValues>(field: K, value: SamplingValues[K]) => void;
    disabled?: boolean;
    size?: "1" | "2";
};
export declare const ModelSamplingParams: React.FC<ModelSamplingParamsProps>;
export {};
