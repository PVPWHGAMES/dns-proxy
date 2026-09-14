import type { BadgeVariant } from "../components/ui/Badge";

/**
 * DNS 查询结果的展示文案与徽章样式
 *
 * 取值与后端 `DnsQueryLog.action` 一一对应，未识别的取值原样展示，
 * 避免所有未知状态都被兜底成某一个文案（曾经把「合并」「失败」都显示成「缓存」）。
 */
const DNS_ACTION_STYLE: Record<string, { label: string; variant: BadgeVariant }> = {
  success: { label: "成功", variant: "success" },
  blocked: { label: "阻止", variant: "danger" },
  cached: { label: "缓存", variant: "info" },
  coalesced: { label: "合并", variant: "teal" },
  failed: { label: "失败", variant: "danger" },
};

/** 取查询结果的中文文案 */
export function dnsActionLabel(action: string): string {
  return DNS_ACTION_STYLE[action]?.label ?? action;
}

/** 取查询结果的徽章样式 */
export function dnsActionVariant(action: string): BadgeVariant {
  return DNS_ACTION_STYLE[action]?.variant ?? "neutral";
}
