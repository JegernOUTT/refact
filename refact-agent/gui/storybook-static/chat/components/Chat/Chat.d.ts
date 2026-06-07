import React from "react";
import { ChatFormProps } from "../ChatForm";
import { type Config } from "../../features/Config/configSlice";
export type ChatProps = {
    host: Config["host"];
    tabbed: Config["tabbed"];
    backFromChat: () => void;
    style?: React.CSSProperties;
    unCalledTools: boolean;
    maybeSendToSidebar: ChatFormProps["onClose"];
};
export declare const Chat: React.FC<ChatProps>;
