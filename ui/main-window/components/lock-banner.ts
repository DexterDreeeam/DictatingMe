/** 保留旧组件契约；Configure 状态不在界面中展示。 */
export function renderLockBanner(): HTMLElement {
  const banner = document.createElement('span');
  banner.className = 'lock-banner';
  banner.hidden = true;
  banner.setAttribute('aria-hidden', 'true');
  return banner;
}
