import "../../src/lib/render/web.css";
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { ImageEditor } from "../../src/components/ImageEditor";

function Fixture() {
  const [file, setFile] = useState<File | null>(null);
  const [output, setOutput] = useState("");
  const open = async () => {
    const canvas = document.createElement("canvas");
    canvas.width = 600;
    canvas.height = 400;
    const ctx = canvas.getContext("2d")!;
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, 600, 400);
    const blob = await new Promise<Blob>((resolve) =>
      canvas.toBlob((value) => resolve(value!)),
    );
    setFile(
      new File([blob], "fixture.png", { type: "image/png", lastModified: 123 }),
    );
  };
  return (
    <>
      <button onClick={() => void open()}>Open editor</button>
      <input
        aria-label="Load fixture file"
        type="file"
        onChange={(event) => setFile(event.target.files?.[0] ?? null)}
      />
      <output data-testid="result">{output}</output>
      {file && (
        <ImageEditor
          file={file}
          onCancel={() => {
            setOutput("cancelled");
            setFile(null);
          }}
          onApply={(result) => {
            void (async () => {
              const bitmap = await createImageBitmap(result);
              const canvas = document.createElement("canvas");
              canvas.width = bitmap.width;
              canvas.height = bitmap.height;
              const ctx = canvas.getContext("2d")!;
              ctx.drawImage(bitmap, 0, 0);
              const pixel = [
                ...ctx.getImageData(
                  Math.round(bitmap.width / 2),
                  Math.round(bitmap.height / 2),
                  1,
                  1,
                ).data,
              ];
              const pixels = ctx.getImageData(
                0,
                0,
                canvas.width,
                canvas.height,
              ).data;
              let redPixels = 0;
              for (let i = 0; i < pixels.length; i += 4)
                if (pixels[i] > 200 && pixels[i + 1] < 80) redPixels++;
              setOutput(
                JSON.stringify({
                  same: result === file,
                  type: result.type,
                  size: result.size,
                  width: bitmap.width,
                  height: bitmap.height,
                  pixel,
                  redPixels,
                }),
              );
              bitmap.close();
              setFile(null);
            })();
          }}
        />
      )}
    </>
  );
}
createRoot(document.getElementById("refact-chat")!).render(
  <Provider store={setUpStore()}>
    <Theme>
      <Fixture />
    </Theme>
  </Provider>,
);
