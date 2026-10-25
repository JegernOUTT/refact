import { parseWorkPage, type WorkPage } from "./workTypes";

export class WorkFrameDecoder {
  private readonly decoder = new TextDecoder();
  private buffer = "";
  private pendingCR = false;

  *push(bytes: Uint8Array): Generator<string> {
    let chunk = this.decoder.decode(bytes, { stream: true });
    if (this.pendingCR) chunk = `\r${chunk}`;
    this.pendingCR = chunk.endsWith("\r");
    if (this.pendingCR) chunk = chunk.slice(0, -1);
    this.buffer += chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
    let boundary = this.buffer.indexOf("\n\n");
    while (boundary >= 0) {
      const block = this.buffer.slice(0, boundary);
      this.buffer = this.buffer.slice(boundary + 2);
      yield block;
      boundary = this.buffer.indexOf("\n\n");
    }
  }
}

export function parseWorkFrame(block: string): WorkPage | null {
  let event = "";
  const data: string[] = [];
  for (const line of block.split("\n")) {
    if (line.startsWith("event:")) event = line.slice(6).trim();
    if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
  }
  if (event === "work_error")
    throw new Error("Work visibility changed or service became unavailable");
  if (data.length === 0) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(data.join("\n"));
  } catch {
    throw new Error("Invalid Work snapshot");
  }
  const page = parseWorkPage(parsed);
  if (!page || event !== "snapshot") throw new Error("Invalid Work snapshot");
  return page;
}
