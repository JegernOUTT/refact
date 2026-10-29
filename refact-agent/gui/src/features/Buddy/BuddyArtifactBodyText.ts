const FENCE = /^\s*(```|~~~)/;
const KEY_VALUE = /\b[a-z][a-z0-9_]*=\S/gi;
const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;
const LONG_HEX = /\b[0-9a-f]{16,}\b/i;
const MACHINE_HEAD = /^[a-z][a-z0-9]*(?:_[a-z0-9]+)+(?:\s|$)/;
const LEADING_HEADING = /^\s*#{1,6}\s+(.+?)\s*#*\s*(?:\r?\n|$)/;

function isMachinePayloadLine(line: string): boolean {
  const text = line.trim();
  if (text === "" || /^[#>*\-+|]/.test(text) || /^\d+\./.test(text))
    return false;
  const pairs = text.match(KEY_VALUE)?.length ?? 0;
  const tokens = text.split(/\s+/);
  let plainRun = 0;
  let machine = 0;
  for (const token of tokens) {
    if (
      /^[a-z][a-z0-9_]*=\S+/i.test(token) ||
      MACHINE_HEAD.test(token) ||
      UUID.test(token) ||
      LONG_HEX.test(token)
    ) {
      machine += 1;
      plainRun = 0;
    } else if (++plainRun >= 3) return false;
  }
  return pairs >= 1 && machine / tokens.length >= 0.75;
}

export function fenceMachinePayloads(body: string): string {
  const lines = body.split(/\r?\n/);
  const output: string[] = [];
  let inFence = false;
  let block: string[] = [];
  const flush = () => {
    if (block.length === 0) return;
    output.push("```text", ...block, "```");
    block = [];
  };
  for (const line of lines) {
    if (FENCE.test(line)) {
      flush();
      inFence = !inFence;
      output.push(line);
      continue;
    }
    if (!inFence && isMachinePayloadLine(line)) {
      block.push(line);
      continue;
    }
    flush();
    output.push(line);
  }
  flush();
  return output.join("\n");
}

function normalize(value: string): string {
  return value.trim().replace(/\s+/g, " ").toLowerCase();
}

export function stripLeadingTitleHeading(body: string, title: string): string {
  const match = LEADING_HEADING.exec(body);
  if (!match || normalize(match[1]) !== normalize(title)) return body;
  return body.slice(match[0].length).trim();
}
