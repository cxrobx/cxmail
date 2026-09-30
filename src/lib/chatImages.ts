/**
 * Pictures attached to a chat message. These limits mirror
 * `email::chat_agent::{MAX_IMAGES_PER_MESSAGE, MAX_IMAGE_BYTES}`; Rust checks
 * again from the bytes, so this is only the friendly early answer.
 */
export const MAX_CHAT_IMAGES = 4;
export const MAX_CHAT_IMAGE_BYTES = 5 * 1024 * 1024;
export const CHAT_IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/** Why a file can't be attached, or null when it can. */
export function imageProblem(file: { type: string; size: number; name?: string }): string | null {
  if (!CHAT_IMAGE_TYPES.includes(file.type)) return "Only PNG, JPEG, GIF and WebP pictures can be attached.";
  if (file.size > MAX_CHAT_IMAGE_BYTES) return `${file.name || "That picture"} is over 5 MB.`;
  return null;
}

export function readAsDataUrl(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error("read failed"));
    reader.readAsDataURL(file);
  });
}
