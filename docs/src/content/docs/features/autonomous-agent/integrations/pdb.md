---
title: PDB Integration
description: Control Python debugger sessions from Refact Agent.
---

The PDB integration exposes the `pdb` tool to the agent. It starts and controls a Python debugger session so the agent can inspect stack frames, variables, and execution flow.

## Setup

Configure the Python interpreter path if Refact should use a specific virtual environment or Python installation. Leave it empty to use the default `python3` command.

## How it works

- Start a session with a command such as `python -m pdb script.py`.
- Send one debugger command at a time, such as `break`, `continue`, `list`, `where`, `print`, or `quit`.
- Use a working directory when the script should run from a specific project path.
- If the debugged process is still running, the agent can wait or stop the session.

## Safety

PDB runs Python code from your project. Use the same caution you would use when running scripts manually, especially if the program touches files, services, or databases.
