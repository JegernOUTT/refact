#!/usr/bin/env node

"use strict";

const { spawnSync } = require("node:child_process");
const path = require("node:path");

const executable = path.join(
  __dirname,
  "..",
  "vendor",
  process.platform === "win32" ? "refact.exe" : "refact",
);
const result = spawnSync(executable, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  console.error(`Unable to start Refact: ${result.error.message}`);
  console.error("Retry installation with `npm rebuild refact-ai`.");
  process.exit(1);
}

if (result.signal) {
  console.error(`Refact exited after signal ${result.signal}.`);
  process.exit(1);
}

process.exit(result.status === null ? 1 : result.status);
