import { type ReactNode } from "react";
import { cn } from "../../lib/utils";

export type BadgeVariant =
  | "success"
  | "danger"
  | "warning"
  | "info"
  | "purple"
  | "orange"
  | "teal"
  | "neutral";

const variants: Record<BadgeVariant, string> = {
  success: "bg-green-100 text-green-700 dark:bg-green-500/15 dark:text-green-400",
  danger: "bg-red-100 text-red-700 dark:bg-red-500/15 dark:text-red-400",
  warning: "bg-yellow-100 text-yellow-700 dark:bg-yellow-500/15 dark:text-yellow-400",
  info: "bg-blue-100 text-blue-700 dark:bg-blue-500/15 dark:text-blue-400",
  purple: "bg-purple-100 text-purple-700 dark:bg-purple-500/15 dark:text-purple-400",
  orange: "bg-orange-100 text-orange-700 dark:bg-orange-500/15 dark:text-orange-400",
  teal: "bg-teal-100 text-teal-700 dark:bg-teal-500/15 dark:text-teal-400",
  neutral: "bg-gray-100 text-gray-700 dark:bg-gray-500/15 dark:text-gray-300",
};

interface BadgeProps {
  variant?: BadgeVariant;
  children: ReactNode;
  className?: string;
}

export default function Badge({ variant = "neutral", children, className }: BadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium",
        variants[variant],
        className,
      )}
    >
      {children}
    </span>
  );
}
