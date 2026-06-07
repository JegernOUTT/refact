export type ExtractedParam = {
    name: string;
    type: string;
    description: string;
};
export declare function extractParamsFromSchema(schema: Record<string, unknown>): ExtractedParam[];
export declare function toInputSchema(params: {
    name: string;
    type: string;
    description: string;
}[], required: string[]): Record<string, unknown>;
export declare function fromInputSchema(schema: Record<string, unknown>): {
    params: {
        name: string;
        type: string;
        description: string;
    }[];
    required: string[];
};
