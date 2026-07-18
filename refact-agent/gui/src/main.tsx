/**
 * Only used by the dev server
 */

import { render } from "./lib";

const element = document.getElementById("refact-chat");
const surface = new URLSearchParams(window.location.search).get("surface");

if (element) {
  render(element, {
    host: "web",
    dev: true,
    engineServed: false,
    lspUrl: "",
    lspPort: 8001,
    features: { statistics: true, vecdb: true, ast: true, codegraph: true },
    themeProps: {},
    ...(surface === "dashboard" ? { surface } : {}),
  });
}
