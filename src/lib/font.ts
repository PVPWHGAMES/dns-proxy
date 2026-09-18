export const FONT_OPTIONS = [
  { value: "Microsoft YaHei", label: "微软雅黑（默认）" },
  { value: "汉仪旗黑65简", label: "汉仪旗黑65简" },
] as const;

/** 兼容早期版本保存的旧字体标识。 */
export function normalizeAppFont(font: string) {
  return font === "Honkai Star Rail" ? "汉仪旗黑65简" : font;
}

export function applyAppFont(font: string) {
  const normalized = normalizeAppFont(font);
  const selected = FONT_OPTIONS.find((option) => option.value === normalized)?.value ?? "Microsoft YaHei";
  document.documentElement.style.setProperty("--app-font", `"${selected}", "Microsoft YaHei", sans-serif`);
  document.documentElement.dataset.appFont = selected;
}
