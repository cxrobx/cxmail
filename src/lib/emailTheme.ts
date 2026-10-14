import { contrast, over, type RGB } from "./contrast";

const DARK_GROUND: RGB = [28, 26, 23];
const LIGHT_INK: RGB = [232, 220, 200];
const DARK_INK: RGB = [30, 25, 15];
const LINK_INK: RGB = [10, 132, 255];
const DARK_LINK_INK: RGB = [0, 88, 190];
const QUOTE_INK: RGB = [160, 144, 120];
const MIN_CONTRAST = 4.5;

interface Colour {
  rgb: RGB;
  alpha: number;
}

/**
 * Resolve CSS colours with the same engine that paints the email. A detached
 * one-pixel canvas handles named colours, hex, rgb/hsl and alpha without adding
 * email HTML to the app document or fetching any of its resources.
 */
function colourReader() {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  const cache = new Map<string, Colour | null>();
  return (value: string): Colour | null => {
    if (!context || !value) return null;
    if (/^(currentcolor|inherit|initial|unset|revert|revert-layer)$/i.test(value)) return null;
    if (cache.has(value)) return cache.get(value)!;
    // Invalid assignments leave fillStyle untouched. Two different sentinels
    // distinguish invalid CSS from a valid colour equal to either sentinel.
    context.fillStyle = "#010203";
    context.fillStyle = value;
    const first = context.fillStyle;
    context.fillStyle = "#040506";
    context.fillStyle = value;
    if (first !== context.fillStyle) {
      cache.set(value, null);
      return null;
    }
    context.clearRect(0, 0, 1, 1);
    context.fillRect(0, 0, 1, 1);
    const [r, g, b, a] = context.getImageData(0, 0, 1, 1).data;
    const colour: Colour = { rgb: [r, g, b], alpha: a / 255 };
    cache.set(value, colour);
    return colour;
  };
}

/**
 * Style an already-sanitized email for dark mode without changing its artwork.
 * Backgrounds belong to the sender: transparent descendants must use the
 * background of their ancestors, not blindly inherit the app's pale ink.
 * Only unreadable foregrounds change; light mode returns the original HTML.
 * This also works in the script-free hover preview's sandbox.
 */
export function prepareEmailHtml(html: string, isDark: boolean): string {
  if (!isDark) return html;
  const doc = new DOMParser().parseFromString(html, "text/html");
  const readColour = colourReader();
  const backgrounds = new Map<Element, RGB>([[doc.body, DARK_GROUND]]);
  const foregrounds = new Map<Element, Colour>([[doc.body, { rgb: LIGHT_INK, alpha: 1 }]]);

  // querySelectorAll is in document order, so each parent has been resolved
  // before its children. Iteration also avoids recursion on deeply nested mail.
  for (const element of doc.body.querySelectorAll<HTMLElement>("*")) {
    const parent = element.parentElement!;
    const parentGround = backgrounds.get(parent) ?? DARK_GROUND;
    const inheritedInk = foregrounds.get(parent) ?? { rgb: LIGHT_INK, alpha: 1 };
    const style = element.style;
    const isLink = element.tagName === "A";
    const defaultInk = isLink ? { rgb: LINK_INK, alpha: 1 }
      : element.tagName === "BLOCKQUOTE" ? { rgb: QUOTE_INK, alpha: 1 } : inheritedInk;
    const inkValue = style.color || element.getAttribute("color") || "";
    let ink = readColour(inkValue) ?? defaultInk;
    if (/^(inherit|unset|currentcolor)$/i.test(inkValue)) ink = inheritedInk;
    if (/^(initial|revert|revert-layer)$/i.test(inkValue)) ink = { rgb: [0, 0, 0], alpha: 1 };
    const backgroundValue = style.backgroundColor || element.getAttribute("bgcolor") || "";
    const currentColourBackground = backgroundValue.toLowerCase() === "currentcolor";
    const authoredGround = currentColourBackground
      ? ink
      : readColour(backgroundValue);
    const ground = authoredGround ? over(authoredGround.rgb, authoredGround.alpha, parentGround) : parentGround;
    backgrounds.set(element, ground);

    let readableInk = over(ink.rgb, ink.alpha, ground);
    if (ink.alpha > 0 && contrast(readableInk, ground) < MIN_CONTRAST) {
      const candidates = isLink
        ? [LINK_INK, DARK_LINK_INK, LIGHT_INK, DARK_INK]
        : [LIGHT_INK, DARK_INK];
      readableInk = candidates.find((candidate) => contrast(candidate, ground) >= MIN_CONTRAST)
        ?? ([0, 0, 0] as RGB);
      if (contrast(readableInk, ground) < MIN_CONTRAST) readableInk = [255, 255, 255];
      // currentColor backgrounds would otherwise change along with the ink,
      // leaving text and background identical after the correction too.
      if (currentColourBackground) {
        style.setProperty("background-color", `rgba(${ink.rgb.join(", ")}, ${ink.alpha})`, "important");
      }
      style.setProperty("color", `rgb(${readableInk.join(", ")})`, "important");
      ink = { rgb: readableInk, alpha: 1 };
    }
    foregrounds.set(element, ink);
  }
  return doc.body.innerHTML;
}
