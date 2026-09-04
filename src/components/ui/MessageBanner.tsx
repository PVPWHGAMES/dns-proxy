import { CheckCircle2, AlertCircle } from "lucide-react";
import { cn } from "../../lib/utils";

interface MessageBannerProps {
  type: "success" | "error";
  text: string;
  className?: string;
}

export default function MessageBanner({ type, text, className }: MessageBannerProps) {
  const Icon = type === "success" ? CheckCircle2 : AlertCircle;

  return (
    <div
      className={cn(
        "flex items-center gap-2 p-4 rounded-lg border text-sm",
        type === "success"
          ? "bg-green-50 border-green-200 text-green-700 dark:bg-green-500/10 dark:border-green-500/20 dark:text-green-400"
          : "bg-red-50 border-red-200 text-red-700 dark:bg-red-500/10 dark:border-red-500/20 dark:text-red-400",
        className,
      )}
    >
      <Icon className="w-4 h-4 shrink-0" />
      <span>{text}</span>
    </div>
  );
}
