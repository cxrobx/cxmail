import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { api } from "@/lib/tauri";
import type { ImagePolicy } from "@/types/email";
import EmailFrame from "../EmailFrame";
vi.mock("@/hooks/useResolvedTheme", () => ({ useResolvedTheme: () => "light" }));
const ref = { accountId: "a", folder: "INBOX", uid: 51 };
const html = '<p style="background-image:url(https://images.example.org/bg.png)">Hello</p><img src="" data-original-src="https://images.example.org/photo.png">';
const good: ImagePolicy = { authentication: { status: "passed", domain: "example.org", provider: "Google", explanation: "Domain authentication is not content safety." }, sender_email: "news@example.org", sender_trusted: true, auto_load: true, suspicious: false, has_remote_images: true, blocked_reason: null };
function frame(container: HTMLElement) { return container.querySelector("iframe")!; }
function imageCsp(container: HTMLElement) { return frame(container).srcdoc.match(/content="img-src [^"]*/)?.[0] ?? ""; }
beforeEach(() => { vi.restoreAllMocks(); });
it("allows a trusted authenticated message and preserves the iframe sandbox", async () => {
  vi.spyOn(api.imageTrust, "policy").mockResolvedValue(good);
  const { container } = render(<EmailFrame html={html} senderEmail="news@example.org" messageRef={ref} />);
  expect(imageCsp(container)).not.toContain("https:");
  await waitFor(() => expect(imageCsp(container)).toContain("https:"));
  expect(frame(container).getAttribute("sandbox")).not.toContain("allow-same-origin");
  expect(screen.queryByRole("button", { name: "Show images" })).toBeNull();
});
it.each([
  { ...good, authentication: { ...good.authentication, status: "failed" as const } },
  { ...good, authentication: { ...good.authentication, status: "unknown" as const } },
  { ...good, suspicious: true },
  { ...good, sender_trusted: false },
  { ...good, sender_email: "other@example.org" },
])("blocks inconsistent or ineligible permission %# including background images", async (policy) => {
  vi.spyOn(api.imageTrust, "policy").mockResolvedValue(policy);
  const { container } = render(<EmailFrame html={html} senderEmail="news@example.org" messageRef={ref} />);
  await waitFor(() => expect(api.imageTrust.policy).toHaveBeenCalled());
  expect(imageCsp(container)).not.toContain("https:");
  expect(screen.getByRole("button", { name: "Show images" })).toBeInTheDocument();
});
it("show once enables images without trusting the sender or changing script policy", async () => {
  vi.spyOn(api.imageTrust, "policy").mockResolvedValue({ ...good, auto_load: false, sender_trusted: false });
  const trust = vi.spyOn(api.imageTrust, "trust");
  const { container } = render(<EmailFrame html={html} senderEmail="news@example.org" messageRef={ref} />);
  fireEvent.click(screen.getByRole("button", { name: "Show images" }));
  await waitFor(() => expect(imageCsp(container)).toContain("https:"));
  expect(trust).not.toHaveBeenCalled();
  expect(frame(container).srcdoc).toContain('<script src="/email-frame.js"></script>');
});
it("re-checks authentication after trusting instead of granting images from the address alone", async () => {
  const policy = vi.spyOn(api.imageTrust, "policy").mockResolvedValue({ ...good, sender_trusted: false, auto_load: false });
  const trust = vi.spyOn(api.imageTrust, "trust").mockResolvedValue(undefined);
  const { container } = render(<EmailFrame html={html} senderEmail="news@example.org" messageRef={ref} />);
  const button = screen.getByRole("button", { name: "Always trust this sender" });
  await waitFor(() => expect(button).toBeEnabled());
  policy.mockResolvedValue({ ...good, auto_load: false, authentication: { ...good.authentication, status: "failed" } });
  fireEvent.click(button);
  await waitFor(() => expect(trust).toHaveBeenCalledWith("news@example.org"));
  await waitFor(() => expect(policy).toHaveBeenCalledTimes(2));
  expect(imageCsp(container)).not.toContain("https:");
});
it("revocation on focus removes automatic image permission", async () => {
  const policy = vi.spyOn(api.imageTrust, "policy").mockResolvedValue(good);
  const { container } = render(<EmailFrame html={html} senderEmail="news@example.org" messageRef={ref} />);
  await waitFor(() => expect(imageCsp(container)).toContain("https:"));
  policy.mockResolvedValue({ ...good, sender_trusted: false, auto_load: false });
  fireEvent(window, new Event("focus"));
  await waitFor(() => expect(imageCsp(container)).not.toContain("https:"));
});
it("an older message check cannot grant permission to the next message from the same sender", async () => {
  let resolve!: (p: ImagePolicy) => void;
  vi.spyOn(api.imageTrust, "policy").mockImplementation((_a, _f, uid) => uid === ref.uid ? new Promise((r) => { resolve = r; }) : Promise.resolve({ ...good, auto_load: false }));
  const { container, rerender } = render(<EmailFrame html={html} senderEmail="news@example.org" messageRef={ref} />);
  await waitFor(() => expect(resolve).toBeDefined());
  rerender(<EmailFrame html={html} senderEmail="news@example.org" messageRef={{ ...ref, uid: 52 }} />);
  resolve(good);
  await waitFor(() => expect(api.imageTrust.policy).toHaveBeenCalledTimes(2));
  expect(imageCsp(container)).not.toContain("https:");
});

it("a trust refresh supersedes an in-flight pre-revocation decision", async () => {
  let resolve!: (p: ImagePolicy) => void;
  const policy = vi.spyOn(api.imageTrust, "policy").mockImplementationOnce(() => new Promise(r => { resolve = r; }))
    .mockResolvedValue({ ...good, auto_load: false, sender_trusted: false });
  const { container } = render(<EmailFrame html={html} senderEmail="news@example.org" messageRef={ref} />);
  await waitFor(() => expect(resolve).toBeDefined());
  fireEvent(window, new Event("cxmail:image-trust-changed"));
  await waitFor(() => expect(policy).toHaveBeenCalledTimes(2));
  resolve(good);
  await waitFor(() => expect(screen.getByRole("button", { name: "Always trust this sender" })).toBeEnabled());
  expect(imageCsp(container)).not.toContain("https:");
});
