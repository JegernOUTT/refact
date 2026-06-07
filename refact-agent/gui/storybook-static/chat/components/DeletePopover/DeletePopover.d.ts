import { FC } from "react";
export type DeletePopoverProps = {
    isDisabled: boolean;
    isDeleting: boolean;
    itemName: string;
    deleteBy: string;
    handleDelete: (deleteBy: string) => void;
};
export declare const DeletePopover: FC<DeletePopoverProps>;
