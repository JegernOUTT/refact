import { type PortOrConnection } from "./chatCommands";
export type ExecStdinResponse = {
    process_id: string;
    status: string;
    bytes_written: number;
    since_seq: number;
    next_seq: number;
    latest_seq: number;
};
export declare function writeProcessStdin(processId: string, chars: string, connection: PortOrConnection, apiKey?: string): Promise<ExecStdinResponse>;
