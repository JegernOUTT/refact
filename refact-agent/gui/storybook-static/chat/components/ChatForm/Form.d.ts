import React from "react";
export declare const Form: React.FC<React.PropsWithChildren<{
    className?: string;
    onClick?: React.MouseEventHandler<HTMLFormElement>;
    onSubmit: React.FormEventHandler<HTMLFormElement>;
    onPointerDownCapture?: React.PointerEventHandler<HTMLFormElement>;
    disabled?: boolean;
}>>;
