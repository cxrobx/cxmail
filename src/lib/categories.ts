export interface CategoryBadgeConfig {
  label: string;
  bgColor: string;
  textColor: string;
}

const CATEGORY_CONFIG: Record<string, CategoryBadgeConfig> = {
  updates: { label: "Updates", bgColor: "rgba(255, 159, 10, 0.15)", textColor: "#ff9f0a" },
  social: { label: "Social", bgColor: "rgba(48, 209, 88, 0.15)", textColor: "#30d158" },
  promotions: { label: "Promos", bgColor: "rgba(191, 90, 242, 0.15)", textColor: "#bf5af2" },
};

export function getCategoryBadge(category: string | null): CategoryBadgeConfig | null {
  if (!category || category === "primary") return null;
  return CATEGORY_CONFIG[category] ?? null;
}
