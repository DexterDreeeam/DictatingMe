/**
 * MainWindow 页面组件的共用接口（首页 + 三个二级页均实现此接口）。
 * 见 brainstrom/plan.md §6。
 */

/** 所有页面组件的挂载/卸载生命周期。 */
export interface PageComponent {
  /** 将页面内容渲染进 container（由 Router 调用）。 */
  mount(container: HTMLElement): void;
  /** 离开该页面前清理（事件监听、订阅等）。 */
  unmount(): void;
}
