# Browser E2E fixtures

These self-contained localhost pages exercise browser interaction, auto-waiting, frames, nested open shadow roots, dialogs, fetches, popups, uploads, downloads, editable content, CSS hover, and strict locator behavior. The Rust harness starts an Axum server on an ephemeral `127.0.0.1` port and does not contact external sites.

Run the ignored suite from `refact-agent/engine` with a Chrome-family binary on `PATH`:

```bash
REFACT_BROWSER_E2E=1 cargo test --test browser_e2e -- --ignored --test-threads=1
```

Set `CHROME=/absolute/path/to/chrome` when the binary is not named `chrome`, `chromium`, `google-chrome`, or `chromium-browser`. The first parity tests describe behavior not yet implemented and can fail until later browser-runtime work lands.
