import type { FC } from "react";
import { IntegrationFieldValue, SchemaToolConfirmation, ToolConfirmation } from "../../../services/refact";
type ConfirmationProps = {
    confirmationByUser: ToolConfirmation | null;
    confirmationFromValues: ToolConfirmation | null;
    defaultConfirmationObject: SchemaToolConfirmation;
    onChange: (fieldKey: string, fieldValue: IntegrationFieldValue) => void;
};
export declare const Confirmation: FC<ConfirmationProps>;
export {};
