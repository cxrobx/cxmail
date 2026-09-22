import { useState } from "react";
import { getInitials } from "@/lib/utils";
import type { Account } from "@/types/email";

// Local icons for known domains (bundled at build time)
const domainIcons: Record<string, string> = {};
const iconModules = import.meta.glob("@/assets/account-icons/*.png", { eager: true, import: "default" }) as Record<string, string>;
for (const [path, url] of Object.entries(iconModules)) {
  const filename = path.split("/").pop()?.replace(".png", "") || "";
  domainIcons[filename] = url;
}

const GmailIcon = ({ size }: { size: number }) => (
  <svg viewBox="0 0 24 24" width={size} height={size}>
    <path d="M24 5.457v13.909c0 .904-.732 1.636-1.636 1.636h-3.819V11.73L12 16.64l-6.545-4.91v9.273H1.636A1.636 1.636 0 0 1 0 19.366V5.457c0-2.023 2.309-3.178 3.927-1.964L5.455 4.64 12 9.548l6.545-4.91 1.528-1.145C21.69 2.28 24 3.434 24 5.457z" fill="#EA4335"/>
  </svg>
);

const AppleIcon = ({ size }: { size: number }) => (
  <svg viewBox="0 0 24 24" width={size} height={size}>
    <path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.8-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z" fill="#A2AAAD"/>
  </svg>
);

const OutlookIcon = ({ size }: { size: number }) => (
  <svg viewBox="0 0 24 24" width={size} height={size}>
    <path d="M24 7.387v10.478c0 .23-.08.424-.238.576-.16.154-.352.232-.578.232h-8.16v-6.08l2.27 1.638a.265.265 0 0 0 .32 0l6.15-4.453c.068-.046.136-.07.18-.07.048 0 .056.024.056.068v1.435c0 .094-.043.164-.13.212l-5.94 4.163a.75.75 0 0 1-.38.11.88.88 0 0 1-.39-.11L15 13.593v8.08h8.184c.226 0 .418-.078.578-.232.158-.152.238-.346.238-.576V7.387z" fill="#0078D4"/>
    <path d="M15.024 21.673V10.892l-1.5-1.186-5.274-3.88v17.182c0 .546.45.99 1.006.99h14.56c.226 0 .418-.078.578-.232.158-.152.238-.346.238-.576v-1.517h-9.608z" fill="#0364B8"/>
    <path d="M8.25 5.826V4.174A.994.994 0 0 0 7.256 3.18H.994A.994.994 0 0 0 0 4.174v15.652c0 .548.446.994.994.994h6.262a.994.994 0 0 0 .994-.994V5.826z" fill="#0078D4"/>
    <ellipse cx="4.119" cy="12" rx="2.381" ry="3.381" fill="none" stroke="#fff" strokeWidth="1"/>
  </svg>
);

function getDomain(email: string): string {
  return email.split("@")[1] || "";
}

function getFaviconUrl(email: string): string {
  const domain = getDomain(email);
  return `https://www.google.com/s2/favicons?domain=${domain}&sz=64`;
}

interface AccountIconProps {
  account: Account;
  size?: number;
}

export default function AccountIcon({ account, size = 28 }: AccountIconProps) {
  const [imgError, setImgError] = useState(false);
  const iconSize = size * 0.6;
  const provider = account.provider?.toLowerCase();

  // Bundled domain icons take priority (custom domains on Google Workspace, etc.)
  const domain = getDomain(account.email);
  const localIcon = domainIcons[domain];
  if (localIcon) {
    return (
      <div className="flex shrink-0 items-center justify-center overflow-hidden rounded-full bg-surface" style={{ width: size, height: size }}>
        <img src={localIcon} alt="" width={iconSize} height={iconSize} className="rounded-sm" draggable={false} />
      </div>
    );
  }

  // Known providers get SVG icons
  if (provider === "gmail") {
    return (
      <div className="flex shrink-0 items-center justify-center rounded-full bg-surface" style={{ width: size, height: size }}>
        <GmailIcon size={iconSize} />
      </div>
    );
  }

  if (provider === "icloud") {
    return (
      <div className="flex shrink-0 items-center justify-center rounded-full bg-surface" style={{ width: size, height: size }}>
        <AppleIcon size={iconSize} />
      </div>
    );
  }

  if (provider === "outlook") {
    return (
      <div className="flex shrink-0 items-center justify-center rounded-full bg-surface" style={{ width: size, height: size }}>
        <OutlookIcon size={iconSize} />
      </div>
    );
  }

  // Other providers: try favicon, fall back to initials
  if (!imgError) {
    return (
      <div className="flex shrink-0 items-center justify-center overflow-hidden rounded-full bg-surface" style={{ width: size, height: size }}>
        <img
          src={getFaviconUrl(account.email)}
          alt=""
          width={iconSize}
          height={iconSize}
          className="rounded-sm"
          draggable={false}
          onError={() => setImgError(true)}
        />
      </div>
    );
  }

  // Fallback: colored initials
  const initials = getInitials(account.display_name, account.email);
  return (
    <div
      className="flex shrink-0 items-center justify-center rounded-full text-[10px] font-medium text-white"
      style={{ width: size, height: size, backgroundColor: account.color || "#0a84ff" }}
    >
      {initials}
    </div>
  );
}
