import {
  File,
  FileArchive,
  FileCode,
  FileCog,
  FileImage,
  FileJson,
  FileText,
} from "lucide-react";
import { describe, expect, it } from "vitest";

import { fileTypeIcon } from "./fileTypeIcon";

describe("fileTypeIcon", () => {
  it.each([
    ["main.rs", FileCode],
    ["component.tsx", FileCode],
    ["package.json", FileJson],
    ["README.md", FileText],
    ["debug.log", FileText],
    ["settings.yaml", FileCog],
    ["Cargo.toml", FileCog],
    ["Dockerfile", FileCog],
    ["photo.png", FileImage],
    ["bundle.zip", FileArchive],
    ["LICENSE", File],
    ["unknown.xyz", File],
  ])("maps %s to the expected icon", (fileName, expectedIcon) => {
    expect(fileTypeIcon(fileName)).toBe(expectedIcon);
  });
});
