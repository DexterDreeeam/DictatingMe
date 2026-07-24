/**
 * 首页导航卡片组件，见 brainstrom/plan.md §6.1。
 */

export interface NavCardProps {
  icon: string;
  title: string;
  subtitle: string;
  onClick: () => void;
}

export function renderNavCard(props: NavCardProps): HTMLElement {
  throw new Error('Not Implemented');
}
