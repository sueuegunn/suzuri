// 画面の配色。トレイメニューの「外観」で選んだものを、ルートの data-theme に light か dark で入れる。
// 「OS に合わせる」のときは OS の設定を見て決め、OS 側で切り替わったらついていく。

import { listen } from "@tauri-apps/api/event";
import { settings, type Theme } from "./engine";

const systemDark = window.matchMedia("(prefers-color-scheme: dark)");
let current: Theme = "system";

function apply() {
  const dark = current === "dark" || (current === "system" && systemDark.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

/** 起動時に一度だけ呼ぶ。設定を読むまでは OS の設定に合わせておく */
export function initTheme() {
  apply();
  systemDark.addEventListener("change", apply);
  settings.getTheme().then((t) => {
    current = t;
    apply();
  });
  listen<Theme>("theme", (e) => {
    current = e.payload;
    apply();
  });
}
