/**
 * 首页：三张导航卡片（InputDevice / EvokeWord / History），见 brainstrom/plan.md §6.1。
 */

import type { PageComponent } from '../page';
import type { RouteName } from '../router';

export interface HomePageDeps {
  /** 卡片点击后请求跳转到目标二级页 */
  onNavigate: (route: RouteName) => void;
}

export class HomePage implements PageComponent {
  constructor(private readonly deps: HomePageDeps) {
    throw new Error('Not Implemented');
  }

  mount(container: HTMLElement): void {
    throw new Error('Not Implemented');
  }

  unmount(): void {
    throw new Error('Not Implemented');
  }
}
