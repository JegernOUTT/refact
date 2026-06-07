import { FC } from "react";
import { Config } from "../Config/configSlice";
type ThreadHistoryProps = {
    onCloseThreadHistory: () => void;
    backFromThreadHistory: () => void;
    host: Config["host"];
    tabbed: Config["tabbed"];
    chatId: string;
};
export declare const ThreadHistory: FC<ThreadHistoryProps>;
export {};
