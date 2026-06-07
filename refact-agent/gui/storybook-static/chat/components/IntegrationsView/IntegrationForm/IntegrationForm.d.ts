import { FC, FormEvent } from "react";
import { IntegrationFieldValue, type Integration } from "../../../services/refact";
type IntegrationFormProps = {
    integrationPath: string;
    isApplying: boolean;
    isDisabled: boolean;
    isDeletingIntegration: boolean;
    handleSubmit: (event: FormEvent<HTMLFormElement>) => void;
    handleDeleteIntegration: (path: string) => void;
    onSchema: (schema: Integration["integr_schema"]) => void;
    onValues: (values: Integration["integr_values"]) => void;
    handleUpdateFormField: (fieldKey: string, fieldValue: IntegrationFieldValue) => void;
    formValues: Integration["integr_values"];
};
export declare const IntegrationForm: FC<IntegrationFormProps>;
export {};
