import { type LucideIcon } from "lucide-react";
import { cn } from "../../lib/utils";

export type StatColor = "blue" | "red" | "yellow" | "green" | "purple" | "orange";

const colors: Record<StatColor, string> = {
  blue: "bg-blue-50 text-blue-600 dark:bg-blue-500/15 dark:text-blue-400",
  red: "bg-red-50 text-red-600 dark:bg-red-500/15 dark:text-red-400",
  yellow: "bg-yellow-50 text-yellow-600 dark:bg-yellow-500/15 dark:text-yellow-400",
  green: "bg-green-50 text-green-600 dark:bg-green-500/15 dark:text-green-400",
  purple: "bg-purple-50 text-purple-600 dark:bg-purple-500/15 dark:text-purple-400",
  orange: "bg-orange-50 text-orange-600 dark:bg-orange-500/15 dark:text-orange-400",
};

interface StatCardProps {
  title: string;
  value: string;
  icon: LucideIcon;
  color: StatColor;
  /** 悬停说明。用于需要澄清口径的指标，避免与任务管理器等外部读数对不上 */
  hint?: string;
}

export default function StatCard({ title, value, icon: Icon, color, hint }: StatCardProps) {
  return (
    <div
      className={`bg-card rounded-xl border p-4${hint ? " cursor-help" : ""}`}
      title={hint}
    >
      <div className={cn("inline-flex p-2 rounded-lg", colors[color])}>
        <Icon className="w-5 h-5" />
      </div>
      <div className="mt-3">
        <p className="text-2xl font-bold tabular-nums">{value}</p>
        <p className="text-sm text-muted-foreground">{title}</p>
      </div>
    </div>
  );
}
