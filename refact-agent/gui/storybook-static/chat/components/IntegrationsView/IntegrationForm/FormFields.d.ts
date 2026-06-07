import { Integration, IntegrationField, IntegrationFieldValue, IntegrationPrimitive } from "../../../services/refact";
import { FC } from "react";
type FormFieldsProps = {
    integration: Integration;
    importantFields: Record<string, IntegrationField<NonNullable<IntegrationPrimitive>>>;
    extraFields: Record<string, IntegrationField<NonNullable<IntegrationPrimitive>>>;
    areExtraFieldsRevealed: boolean;
    onChange: (fieldKey: string, fieldValue: IntegrationFieldValue) => void;
    values: Integration["integr_values"];
};
export declare const FormFields: FC<FormFieldsProps>;
export {};
