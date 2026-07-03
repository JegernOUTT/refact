/**
 * Only used by the dev server
 */

import { render } from "./lib";

const element = document.getElementById("refact-chat");

if (element) {
  render(element, {
    host: "web",
    dev: true,
    engineServed: false,
    lspUrl: "",
    lspPort: 8001,
    features: { statistics: true, vecdb: true, ast: true, codegraph: true },
    themeProps: {},
  });
}
