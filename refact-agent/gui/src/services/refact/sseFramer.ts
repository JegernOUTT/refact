export class SseFramer {
  private segments: string[] = [];
  private lineLength = 0;
  private skipLF = false;

  *push(chunk: string): Generator<string> {
    let start = 0;
    for (let i = 0; i < chunk.length; i++) {
      const code = chunk.charCodeAt(i);
      if (this.skipLF) {
        this.skipLF = false;
        if (code === 10) {
          start = i + 1;
          continue;
        }
      }
      if (code !== 10 && code !== 13) continue;

      if (i > start) this.segments.push(chunk.slice(start, i));
      this.lineLength += i - start;
      start = i + 1;
      this.skipLF = code === 13;
      if (this.lineLength === 0) {
        if (this.segments.length > 0) {
          this.segments.pop();
          const frame = this.segments.join("");
          this.segments = [];
          yield frame;
        }
      } else {
        this.segments.push("\n");
        this.lineLength = 0;
      }
    }
    if (start < chunk.length) {
      this.segments.push(chunk.slice(start));
      this.lineLength += chunk.length - start;
    }
  }
}
