import { FC } from "react";
import { Integration, IntegrationFieldValue } from "../../../services/refact";
type FormAvailabilityAndDeleteProps = {
    integration: Integration;
    isApplying: boolean;
    isDeletingIntegration: boolean;
    onDelete: (path: string) => void;
    onChange: (fieldKey: string, fieldValue: IntegrationFieldValue) => void;
    formValues: Record<string, IntegrationFieldValue> | null;
};
export declare const FormAvailabilityAndDelete: FC<FormAvailabilityAndDeleteProps>;
export {};
