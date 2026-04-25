---
title: Chrome Tool
description: Use the Chrome Tool for browser automation and web interactions
---

The Chrome Tool enables the Refact.ai Agent to interact with browsers like Chrome, Chromium, and Edge. It is a built-in tool — no configuration is required. The agent automatically detects Chrome on your system.

## Availability

Chrome is available in all main agent modes: Agent, Quick Agent, Debug, Explore, Learn, Review, and Plan.

## Functionality

The Chrome Tool lets the agent open browser tabs, navigate to URLs, take screenshots, interact with page elements, and extract page content. It supports desktop, mobile, and tablet emulation.

## Supported Commands

| Command | Description |
|---------|-------------|
| `open_tab <tab_id> <desktop\|mobile\|tablet>` | Open a new browser tab |
| `navigate_to <tab_id> <url>` | Navigate to a URL |
| `screenshot <tab_id>` | Capture a screenshot |
| `html <tab_id> <selector>` | Get HTML of an element |
| `click_at_element <tab_id> <selector>` | Click an element |
| `fill_field <tab_id> <selector> <text>` | Fill a form field |
| `type_text_at <tab_id> <text>` | Type text at the focused element |
| `press_key <tab_id> <Key>` | Press a keyboard key |
| `scroll_to <tab_id> <selector>` | Scroll to an element |
| `eval <tab_id> <expression>` | Execute JavaScript |
| `styles <tab_id> <selector>` | Get CSS styles |
| `tab_log <tab_id>` | Get browser console log |
| `wait_for <tab_id> <seconds>` | Wait for a duration |
| `wait_for_selector <tab_id> <selector>` | Wait for an element to appear |
| `wait_for_navigation <tab_id>` | Wait for page navigation to complete |
| `list_tabs` | List all open tabs |
| `close_tab <tab_id>` | Close a tab |

## Chrome Installation

If Chrome is not found automatically, install [Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/) or set the path via your system configuration.

Typical Chrome paths:
- Windows: `C:\Program Files\Google\Chrome\Application\chrome.exe`
- macOS: `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`
- Linux: `/usr/bin/google-chrome` or `/usr/bin/chromium`
