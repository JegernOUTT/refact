import { safeDimensions } from "./model";

function hasAnimationChunk(bytes: Uint8Array) {
  const view = new DataView(bytes.buffer);
  for (let offset = 8; offset + 12 <= bytes.length; ) {
    const length = view.getUint32(offset);
    if (length > bytes.length - offset - 12) break;
    if (
      String.fromCharCode(...bytes.subarray(offset + 4, offset + 8)) === "acTL"
    )
      return true;
    offset += length + 12;
  }
  return false;
}

function exifOrientation(
  view: DataView,
  start: number,
  end: number,
  rawTiff = false,
) {
  if (
    !rawTiff &&
    (start + 14 > end ||
      view.getUint32(start) !== 0x45786966 ||
      view.getUint16(start + 4) !== 0)
  )
    return 1;
  const tiff = rawTiff ? start : start + 6;
  if (tiff + 8 > end) return 1;
  const order = view.getUint16(tiff);
  if (order !== 0x4949 && order !== 0x4d4d) return 1;
  const little = order === 0x4949;
  if (view.getUint16(tiff + 2, little) !== 42) return 1;
  const ifd = tiff + view.getUint32(tiff + 4, little);
  if (ifd < tiff + 8 || ifd + 2 > end) return 1;
  const count = view.getUint16(ifd, little);
  if (count > Math.floor((end - ifd - 2) / 12)) return 1;
  for (let i = 0; i < count; i++) {
    const entry = ifd + 2 + i * 12;
    if (
      view.getUint16(entry, little) === 0x112 &&
      view.getUint16(entry + 2, little) === 3 &&
      view.getUint32(entry + 4, little) === 1
    ) {
      const value = view.getUint16(entry + 8, little);
      return value >= 1 && value <= 8 ? value : 1;
    }
  }
  return 1;
}

export async function inspectImage(file: File) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const view = new DataView(bytes.buffer);
  const ascii = (offset: number, count: number) =>
    String.fromCharCode(...bytes.subarray(offset, offset + count));
  let width = 0,
    height = 0,
    animationWarning = false;
  if (ascii(1, 3) === "PNG" && bytes.length >= 24) {
    width = view.getUint32(16);
    height = view.getUint32(20);
    animationWarning = hasAnimationChunk(bytes);
    for (let offset = 8; offset + 12 <= bytes.length; ) {
      const length = view.getUint32(offset);
      if (length > bytes.length - offset - 12) break;
      if (ascii(offset + 4, 4) === "eXIf") {
        if (exifOrientation(view, offset + 8, offset + 8 + length, true) >= 5)
          [width, height] = [height, width];
        break;
      }
      offset += length + 12;
    }
  } else if (ascii(0, 3) === "GIF" && bytes.length >= 10) {
    width = view.getUint16(6, true);
    height = view.getUint16(8, true);
    animationWarning = true;
  } else if (ascii(0, 2) === "BM" && bytes.length >= 26) {
    width = Math.abs(view.getInt32(18, true));
    height = Math.abs(view.getInt32(22, true));
  } else if (bytes[0] === 255 && bytes[1] === 216) {
    let offset = 2,
      orientation = 1;
    while (offset + 1 < bytes.length) {
      if (bytes[offset++] !== 255) break;
      while (bytes[offset] === 255) offset++;
      const marker = bytes[offset++];
      if (marker === 217 || marker === 218) break;
      if (marker === 1 || (marker >= 208 && marker <= 215)) continue;
      if (offset + 2 > bytes.length) break;
      const length = view.getUint16(offset);
      if (length < 2 || offset + length > bytes.length) break;
      if (marker === 225)
        orientation = exifOrientation(view, offset + 2, offset + length);
      if (
        [
          192, 193, 194, 195, 197, 198, 199, 201, 202, 203, 205, 206, 207,
        ].includes(marker) &&
        length >= 8
      ) {
        height = view.getUint16(offset + 3);
        width = view.getUint16(offset + 5);
      }
      offset += length;
    }
    if (orientation >= 5) [width, height] = [height, width];
  } else if (
    ascii(0, 4) === "RIFF" &&
    ascii(8, 4) === "WEBP" &&
    bytes.length >= 30
  ) {
    const type = ascii(12, 4);
    if (type === "VP8X") {
      width = 1 + bytes[24] + (bytes[25] << 8) + (bytes[26] << 16);
      height = 1 + bytes[27] + (bytes[28] << 8) + (bytes[29] << 16);
      animationWarning = (bytes[20] & 2) !== 0;
    } else if (type === "VP8 ") {
      width = view.getUint16(26, true) & 16383;
      height = view.getUint16(28, true) & 16383;
    } else if (type === "VP8L") {
      const bits = view.getUint32(21, true);
      width = (bits & 16383) + 1;
      height = ((bits >>> 14) & 16383) + 1;
    }
  }
  if (!width || !height)
    throw new Error(
      "Cannot safely determine image dimensions. You can attach the original without editing.",
    );
  if (width > 32768 || height > 32768 || width * height > 64_000_000)
    throw new Error(
      "This image exceeds the decoding limit (64 megapixels / 32768 pixels per side). You can attach the original without editing.",
    );
  return {
    width,
    height,
    safe: safeDimensions(width, height),
    animationWarning,
  };
}
