import type { LucideIcon } from "lucide-react";

interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  description?: string;
}

export default function EmptyState({ icon: Icon, title, description }: EmptyStateProps) {
  return (
    <div className="flex h-full flex-col items-center justify-center text-content-muted">
      <Icon className="mb-3 h-10 w-10 opacity-50" />
      <p className="text-sm font-medium">{title}</p>
      {description && <p className="mt-1 text-xs">{description}</p>}
    </div>
  );
}
