# Browser and Chrome Automation

This page is a user-facing summary of Refact browser automation. It is not an implementation specification.

Refact includes a built-in Chrome tool that lets agent workflows inspect and interact with web pages. The tool is useful for debugging web apps, checking UI behavior, extracting page data, reproducing browser issues, and collecting screenshots or console output for the model.

## What it can do

The agent can:

- Launch Chrome or Chromium, connect to an existing Chrome DevTools Protocol endpoint, or reuse a browser session for the chat.
- Open, switch, list, and close tabs.
- Navigate pages, reload, go back, and go forward.
- Click, hover, focus, fill, clear, select, check, and uncheck elements.
- Wait for selectors, navigation, URL changes, text, network idle, hidden elements, or stable elements.
- Capture page and element screenshots.
- Extract text, HTML, attributes, links, tables, DOM snapshots, and accessibility snapshots.
- Run JavaScript expressions, inspect styles, dismiss overlays, highlight elements, and read browser console logs.
- Use desktop, mobile, or tablet viewport settings when opening tabs.

## How to use it

Use an agent-capable mode and ask for a browser task, for example:

- "Open the local app, reproduce the login error, and capture console logs."
- "Check the checkout page on a mobile viewport and summarize visual issues."
- "Extract the table from this documentation page as JSON."
- "Navigate to the issue, take a screenshot, and inspect the failing selector."

The agent chooses browser actions as tool calls and returns results in chat. Screenshots and extracted content can become model context, so avoid sensitive pages unless the selected model and provider are appropriate for that data.

## Configuration

Most users do not need extra setup. If Chrome is not detected, configure a Chrome or Chromium executable path in the Chrome tool settings. Advanced setups can provide a `ws://` DevTools endpoint or a container-exposed browser endpoint.

Settings can also control browser timeout, headless mode, and viewport sizes for desktop, mobile, and tablet sessions.

## Locators and reliability

Browser automation is most reliable when pages have stable selectors. Prefer accessible labels, roles, test ids, unique CSS selectors, and predictable text. Ask the agent to wait for page loads or specific elements when testing single-page applications.

## Safety and privacy

Browser automation may expose page content, URLs, console logs, screenshots, and form input values to the configured model. Use test accounts, local environments, and least-privilege credentials when possible. Tool confirmations and mode settings decide how browser actions are approved.

## Limitations

Some sites block automation or have content that is difficult to inspect, such as cross-origin iframes, heavy canvas UIs, anti-bot flows, or pages requiring hardware-backed authentication. In those cases, provide screenshots, logs, or manual reproduction steps to help the agent continue.
