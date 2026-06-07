import React from "react";
import type { Config } from "../Config/configSlice";
export type ChatProps = {
    host: Config["host"];
    tabbed: Config["tabbed"];
    style?: React.CSSProperties;
    backFromChat: () => void;
};
export declare const Chat: React.FC<ChatProps>;
