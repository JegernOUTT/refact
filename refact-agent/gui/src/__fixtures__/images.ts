/**
 * Canonical fixture image for the GUI image pipeline (audit N-33/N-40/N-64/L-25).
 *
 * Stories that used to render a 1x1 transparent pixel — or no `<img>` at all —
 * now get a REAL 480x270 PNG, so aspect handling, `object-fit`, thumbnail
 * cropping and the lightbox zoom are actually exercised.
 *
 * The PNG bytes are produced here rather than pasted as a multi-kilobyte
 * literal: the encoder below emits a spec-correct 1-bit palette PNG (stored
 * deflate blocks, real CRC32/Adler-32, hand-rolled base64), which keeps the
 * source reviewable while still yielding genuine image data that any decoder
 * reads back at 480x270.
 */

export const FIXTURE_IMAGE_WIDTH = 480;
export const FIXTURE_IMAGE_HEIGHT = 270;
export const FIXTURE_IMAGE_MIME = "image/png";
/** 16:9 — matches the 480x270 canvas. */
export const FIXTURE_IMAGE_ASPECT_RATIO =
  FIXTURE_IMAGE_WIDTH / FIXTURE_IMAGE_HEIGHT;

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const CHECKER_BLOCK_W = 40;
const CHECKER_BLOCK_H = 45;
const BASE64_ALPHABET =
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

function u32(value: number): number[] {
  return [
    (value >>> 24) & 0xff,
    (value >>> 16) & 0xff,
    (value >>> 8) & 0xff,
    value & 0xff,
  ];
}

function crc32(bytes: number[]): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function adler32(bytes: number[]): number {
  let a = 1;
  let b = 0;
  for (const byte of bytes) {
    a = (a + byte) % 65521;
    b = (b + a) % 65521;
  }
  return ((b << 16) | a) >>> 0;
}

function chunk(type: string, data: number[]): number[] {
  const body = [...Array.from(type, (char) => char.charCodeAt(0)), ...data];
  return [...u32(data.length), ...body, ...u32(crc32(body))];
}

/** zlib stream built from uncompressed ("stored") deflate blocks. */
function zlibStored(raw: number[]): number[] {
  const out: number[] = [0x78, 0x01];
  const maxBlock = 0xffff;
  let position = 0;
  do {
    const length = Math.min(maxBlock, raw.length - position);
    const isFinal = position + length >= raw.length ? 1 : 0;
    out.push(
      isFinal,
      length & 0xff,
      (length >>> 8) & 0xff,
      ~length & 0xff,
      (~length >>> 8) & 0xff,
    );
    for (let i = 0; i < length; i += 1) out.push(raw[position + i]);
    position += length;
  } while (position < raw.length);
  out.push(...u32(adler32(raw)));
  return out;
}

/**
 * 1-bit palette scanlines forming a 40x45 checkerboard, so downscaling,
 * cropping and letterboxing are all obvious at a glance.
 */
function buildScanlines(): number[] {
  const bytesPerRow = FIXTURE_IMAGE_WIDTH / 8;
  const raw: number[] = [];
  for (let y = 0; y < FIXTURE_IMAGE_HEIGHT; y += 1) {
    raw.push(0); // filter type: none
    const bandFlipped = Math.floor(y / CHECKER_BLOCK_H) % 2 === 1;
    for (let byteIndex = 0; byteIndex < bytesPerRow; byteIndex += 1) {
      const columnLit = Math.floor((byteIndex * 8) / CHECKER_BLOCK_W) % 2 === 0;
      raw.push(columnLit !== bandFlipped ? 0xff : 0x00);
    }
  }
  return raw;
}

/** Dependency-free base64 (no `btoa`/`Buffer`, so node and browser agree). */
function toBase64(bytes: number[]): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const remaining = bytes.length - i;
    const b0 = bytes[i];
    const b1 = remaining > 1 ? bytes[i + 1] : 0;
    const b2 = remaining > 2 ? bytes[i + 2] : 0;
    out += BASE64_ALPHABET[b0 >> 2];
    out += BASE64_ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)];
    out +=
      remaining > 1 ? BASE64_ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] : "=";
    out += remaining > 2 ? BASE64_ALPHABET[b2 & 0x3f] : "=";
  }
  return out;
}

function buildFixturePng(): string {
  const ihdr = [
    ...u32(FIXTURE_IMAGE_WIDTH),
    ...u32(FIXTURE_IMAGE_HEIGHT),
    1, // bit depth
    3, // colour type: indexed palette
    0, // compression method
    0, // filter method
    0, // interlace method
  ];
  // Two entries: deep slate + indigo highlight.
  const palette = [0x1f, 0x2a, 0x44, 0x6e, 0x9a, 0xff];
  const bytes = [
    ...PNG_SIGNATURE,
    ...chunk("IHDR", ihdr),
    ...chunk("PLTE", palette),
    ...chunk("IDAT", zlibStored(buildScanlines())),
    ...chunk("IEND", []),
  ];
  return toBase64(bytes);
}

/** Raw base64 payload with no data-URI prefix — the shape tool results carry. */
export const FIXTURE_IMAGE_BASE64 = buildFixturePng();

/** Ready-to-render `src` for `<img>` / `DialogImage`. */
export const FIXTURE_IMAGE_DATA_URI = `data:${FIXTURE_IMAGE_MIME};base64,${FIXTURE_IMAGE_BASE64}`;
