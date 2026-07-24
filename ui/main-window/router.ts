/**
 * MainWindow 的极简路由：首页 ⇄ 三个二级页（见 brainstrom/plan.md §6 导航结构）。
 * 打开任意二级页或首页，MainWindow 均处于打开状态 -> Runtime 处于 Configure（§8.4）。
 */

import type { PageComponent } from './page';

export type RouteName = 'home' | 'input-device' | 'evoke-word' | 'history';

export class Router {
  constructor(private readonly container: HTMLElement) {
    throw new Error('Not Implemented');
  }

  /** 注册某个路由对应的页面组件工厂（惰性构造，切换时才实例化）。 */
  register(route: RouteName, factory: () => PageComponent): void {
    throw new Error('Not Implemented');
  }

  /** 切换到指定路由：卸载当前页面组件，挂载目标页面组件。 */
  navigate(route: RouteName): void {
    throw new Error('Not Implemented');
  }

  current(): RouteName {
    throw new Error('Not Implemented');
  }
}
