/**
 * MainWindow 的极简路由：首页 ⇄ 三个二级页（见 brainstrom/plan.md §6 导航结构）。
 * 打开任意二级页或首页，MainWindow 均处于打开状态 -> Runtime 处于 Configure（§8.4）。
 */

import type { PageComponent } from './page';

export type RouteName = 'home' | 'input-device' | 'evoke-word' | 'speech-model' | 'history';

export class Router {
  readonly #factories = new Map<RouteName, () => PageComponent>();
  #activeComponent: PageComponent | null = null;
  #activeRoute: RouteName | null = null;

  constructor(private readonly container: HTMLElement) {
    if (!(container instanceof HTMLElement)) {
      throw new TypeError('Router requires a valid HTMLElement container.');
    }
  }

  /** 注册某个路由对应的页面组件工厂（惰性构造，切换时才实例化）。 */
  register(route: RouteName, factory: () => PageComponent): void {
    if (this.#factories.has(route)) {
      throw new Error(`Route "${route}" is already registered.`);
    }
    this.#factories.set(route, factory);
  }

  /** 切换到指定路由：卸载当前页面组件，挂载目标页面组件。 */
  navigate(route: RouteName): void {
    if (this.#activeRoute === route) {
      return;
    }

    const factory = this.#factories.get(route);
    if (!factory) {
      throw new Error(`Cannot navigate to unregistered route "${route}".`);
    }

    const nextComponent = factory();
    if (!nextComponent || typeof nextComponent.mount !== 'function' || typeof nextComponent.unmount !== 'function') {
      throw new Error(`Factory for route "${route}" did not return a PageComponent.`);
    }

    this.#activeComponent?.unmount();
    this.container.replaceChildren();
    this.#activeComponent = null;
    this.#activeRoute = null;

    try {
      nextComponent.mount(this.container);
      this.#activeComponent = nextComponent;
      this.#activeRoute = route;
      this.container.dataset.route = route;
    } catch (error) {
      try {
        nextComponent.unmount();
      } catch (cleanupError) {
        console.error(`Failed to clean up route "${route}" after a mount error:`, cleanupError);
      }
      this.container.replaceChildren();
      delete this.container.dataset.route;
      throw new Error(`Failed to mount route "${route}".`, { cause: error });
    }
  }

  current(): RouteName {
    if (this.#activeRoute === null) {
      throw new Error('Router has no current route. Call navigate() first.');
    }
    return this.#activeRoute;
  }
}
