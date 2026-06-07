import { FC } from "react";
import type { ProviderFormValues } from "./useProviderForm";
export type FormFieldsProps = {
    providerData: ProviderFormValues;
    fields: Record<string, string | boolean>;
    onChange: (updatedProviderData: ProviderFormValues) => void;
};
export declare const FormFields: FC<FormFieldsProps>;
