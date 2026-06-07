import type { IntegrationField, IntegrationPrimitive } from "../../../services/refact";
export declare const useFormFields: (fields?: Record<string, IntegrationField<NonNullable<IntegrationPrimitive>>>) => {
    importantFields: Record<string, IntegrationField<NonNullable<IntegrationPrimitive>>>;
    extraFields: Record<string, IntegrationField<NonNullable<IntegrationPrimitive>>>;
    areExtraFieldsRevealed: boolean;
    toggleExtraFields: () => void;
};
