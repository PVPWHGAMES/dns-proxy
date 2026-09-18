import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import ErrorBoundary from "./components/ErrorBoundary";
import { api } from "./lib/api";
import { applyAppFont } from "./lib/font";
import "./styles/globals.css";

void api.getConfig().then((config) => applyAppFont(config.app_font)).catch(() => {
  // 配置加载失败时保留 CSS 中的微软雅黑默认值。
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
