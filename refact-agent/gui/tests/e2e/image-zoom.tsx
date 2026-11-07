import "../../src/lib/render/web.css";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { ImageFilePreview } from "../../src/features/Workspace/FilesPanel/ImageFilePreview";

const canvas = document.createElement("canvas");
canvas.width = 1600;
canvas.height = 1200;
const context = canvas.getContext("2d");
if (!context) throw new Error("Missing canvas context");
context.fillStyle = "#bbb";
context.fillRect(0, 0, canvas.width, canvas.height);
context.fillStyle = "#222";
for (let x = 0; x < canvas.width; x += 100)
  for (let y = 0; y < canvas.height; y += 100) context.fillRect(x, y, 20, 20);

const root = document.getElementById("refact-chat");
if (!root) throw new Error("Missing fixture root");
createRoot(root).render(
  <Provider store={setUpStore()}>
    <Theme>
      <div id="image-zoom-fixture">
        <ImageFilePreview alt="zoom-grid.png" src={canvas.toDataURL()} />
      </div>
    </Theme>
  </Provider>,
);
