function stripShikiBackground(html: string): string {
  return html
    .replace(/style="[^"]*background-color:[^;"]*;?/gi, 'style="')
    .replace(/style="[^"]*background:[^;"]*;?/gi, 'style="')
    .replace(/style="\s*"/g, "");
}

export function extractCodeLines(html: string): string[] {
  const codeMatch = /<code[^>]*>([\s\S]*?)<\/code>/i.exec(html);
  const codeHtml = codeMatch ? codeMatch[1] : html;
  const lineStartPattern = /<span\s+class=["'][^"']*\bline\b[^"']*["'][^>]*>/g;
  const starts = [...codeHtml.matchAll(lineStartPattern)].map((match) => ({
    index: match.index,
    length: match[0].length,
  }));

  if (starts.length === 0) {
    return stripShikiBackground(codeHtml).split("\n");
  }

  return starts.map((start, index) => {
    const contentStart = start.index + start.length;
    const nextStart = starts[index + 1]?.index ?? codeHtml.length;
    const rawLine = codeHtml
      .slice(contentStart, nextStart)
      .replace(/<\/span>\s*$/, "");
    return stripShikiBackground(rawLine);
  });
}
