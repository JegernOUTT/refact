import { reactHooksModuleName } from '@reduxjs/toolkit/query/react';
import { Api, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta, QueryDefinition, MutationDefinition, coreModuleName } from '@reduxjs/toolkit/query';
export declare const modelsApi: Api<BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, {
    getModels: QueryDefinition<GetModelsArgs, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", ModelsResponse, "models">;
    getModel: QueryDefinition<GetModelArgs, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", Model, "models">;
    getModelDefaults: QueryDefinition<GetModelDefaultsArgs, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", Model, "models">;
    getCompletionModelFamilies: QueryDefinition<undefined, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", CompletionModelFamiliesResponse, "models">;
    updateModel: MutationDefinition<UpdateModelRequestBody, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", unknown, "models">;
    deleteModel: MutationDefinition<DeleteModelRequestBody, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "MODELS" | "MODEL", unknown, "models">;
}, "models", "MODELS" | "MODEL", typeof coreModuleName | typeof reactHooksModuleName>;
export type SimplifiedModel = {
    name: string;
    enabled: boolean;
    removable: boolean;
    user_configured: boolean;
};
export type ModelsResponse = {
    completion_models: SimplifiedModel[];
    chat_models: SimplifiedModel[];
    embedding_model: SimplifiedModel | null;
};
export type ModelType = "embedding" | "completion" | "chat";
export type GetModelArgs = {
    modelName: string;
    providerName: string;
    modelType: ModelType;
};
export type GetModelDefaultsArgs = Omit<GetModelArgs, "modelName">;
export type GetModelsArgs = {
    providerName: string;
};
export type UpdateModelRequestBody = {
    provider: string;
    model: Model;
    type: ModelType;
};
export type DeleteModelRequestBody = Omit<UpdateModelRequestBody, "model"> & {
    model: string;
};
export type CodeChatModel = {
    n_ctx: number;
    name: string;
    tokenizer: string;
    id: string;
    supports_tools: boolean;
    supports_multimodality: boolean;
    supports_clicks: boolean;
    supports_agent: boolean;
    reasoning_effort_options?: string[] | null;
    supports_thinking_budget?: boolean;
    supports_adaptive_thinking_budget?: boolean;
    default_temperature: number | null;
    default_frequency_penalty?: number | null;
    default_max_tokens?: number | null;
    max_output_tokens?: number | null;
    enabled: boolean;
    type: "chat";
};
export type CodeCompletionModel = {
    n_ctx: number;
    name: string;
    model_family: string | null;
    type: "completion";
    enabled: boolean;
};
export type EmbeddingModel = {
    n_ctx: number;
    name: string;
    id: string;
    tokenizer: string;
    embedding_size: number;
    rejection_threshold: number;
    embedding_batch: number;
    enabled: boolean;
    type: "embedding";
};
export declare function isModelsResponse(data: unknown): data is ModelsResponse;
export type Model = CodeChatModel | CodeCompletionModel | EmbeddingModel;
export declare function isCodeChatModel(data: unknown): data is CodeChatModel;
export declare function isCodeCompletionModel(data: unknown): data is CodeCompletionModel;
export declare function isEmbeddingModel(data: unknown): data is EmbeddingModel;
export declare function isModel(data: unknown): data is Model;
export type CompletionModelFamiliesResponse = {
    model_families: string[];
};
export declare function isCompletionModelFamiliesResponse(data: unknown): data is CompletionModelFamiliesResponse;
