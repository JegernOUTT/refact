---
title: Chrome and Browser Automation
description: Use the built-in Chrome tool for browser automation and page inspection.
---

Chrome is a built-in Refact tool for browser automation. It can launch a Chrome or Chromium process, connect to an existing Chrome DevTools Protocol endpoint, or reuse a browser session already associated with the chat.

## What the agent can do

The browser tool supports workflows such as:

- Open, switch, list, and close tabs.
- Navigate, reload, go back, and go forward.
- Click, hover, focus, fill, clear, select, check, and uncheck page elements.
- Wait for selectors, navigation, URL changes, text, network idle, or stable elements.
- Capture screenshots of the page or a specific element.
- Extract text, HTML, attributes, links, tables, DOM snapshots, and accessibility snapshots.
- Run JavaScript expressions, inspect CSS styles, dismiss overlays, and read console logs.

## Locators

The agent can target elements by CSS, id, name, text, label, role, XPath, placeholder, autocomplete, or test id. Prefer stable locators such as labels, roles, test ids, and unique CSS selectors.

## Configuration

Most users do not need to configure Chrome. If Chrome is not detected, set the Chrome path in tool settings. Advanced setups can connect to a `ws://` CDP endpoint or a container-exposed browser endpoint. Desktop, mobile, and tablet viewport settings are available for responsive testing.

## Privacy and safety

Browser screenshots, DOM content, console logs, and form input values can become model context when the agent uses the tool. Avoid navigating to pages that contain secrets unless the selected model and provider are appropriate for that data. Browser actions can be controlled by the current mode and confirmation settings.
