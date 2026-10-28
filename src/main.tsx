import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import Settings from "./Settings";
import { initTheme } from "./theme";
import "./styles.css";

// 同じページを、オーバーレイと設定ウィンドウ（#settings）の両方で使う
const isSettings = location.hash === "#settings";
if (isSettings) document.body.classList.add("settings-body");
initTheme();

createRoot(document.getElementById("root")!).render(
  <StrictMode>{isSettings ? <Settings /> : <App />}</StrictMode>,
);
