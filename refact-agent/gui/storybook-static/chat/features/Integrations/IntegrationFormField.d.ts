import { FC } from "react";
import { IntegrationFieldValue, type Integration, type IntegrationField, type IntegrationPrimitive } from "../../services/refact";
type IntegrationFormFieldProps = {
    field: IntegrationField<NonNullable<IntegrationPrimitive>>;
    values: Integration["integr_values"];
    fieldKey: string;
    integrationName: string;
    integrationPath: string;
    integrationProject: string;
    isFieldVisible?: boolean;
    onChange: (fieldKey: string, fieldValue: IntegrationFieldValue) => void;
};
export declare const IntegrationFormField: FC<IntegrationFormFieldProps>;
export {};
