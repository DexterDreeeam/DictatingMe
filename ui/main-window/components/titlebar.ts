/**
 * 自定义无边框标题栏：左上角应用名 "Dictating Me" + 右上角播放/电源按钮，
 * 替代 OS 原生标题栏（`decorations: false`），见 brainstrom/plan.md §6。
 */

export interface TitleBarProps {
  /** 固定显示 "Dictating Me" */
  appName: string;
  /** 点击播放按钮：进入后台运行，调用 `requestBackground()`（等价于关闭 MainWindow） */
  onBackground: () => void;
  /** 点击电源按钮：无需确认，调用 `quitApp()` 直接终止整个 Runtime 进程 */
  onQuit: () => void;
}

export function renderTitleBar(props: TitleBarProps): HTMLElement {
  throw new Error('Not Implemented');
}
