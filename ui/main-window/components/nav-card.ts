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
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'nav-card';
  button.addEventListener('click', props.onClick);

  const leading = document.createElement('span');
  leading.className = 'nav-card__leading';

  const icon = document.createElement('span');
  icon.className = 'nav-card__icon';
  icon.setAttribute('aria-hidden', 'true');
  icon.append(createNavIcon(props.icon));

  const title = document.createElement('span');
  title.className = 'nav-card__title';
  title.textContent = props.title;
  leading.append(icon, title);

  const subtitle = document.createElement('span');
  subtitle.className = 'nav-card__subtitle';
  subtitle.textContent = props.subtitle;

  button.append(leading, subtitle);
  return button;
}

function createNavIcon(name: string): SVGSVGElement {
  const namespace = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(namespace, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '1.6');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  const paths = name === 'microphone'
    ? [
        ['path', 'M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z'],
        ['path', 'M19 10v2a7 7 0 0 1-14 0v-2'],
        ['path', 'M12 19v4M8 23h8'],
      ]
    : name === 'bell'
      ? [
          ['path', 'M18 8a6 6 0 0 0-12 0c0 7-3 9-3 9h18s-3-2-3-9'],
          ['path', 'M13.73 21a2 2 0 0 1-3.46 0'],
        ]
      : name === 'wave'
        ? [
            ['path', 'M4 8v8M8 5v14M12 9v6M16 3v18M20 7v10'],
          ]
        : [
          ['circle', '12,12,9'],
          ['path', 'M12 7v5l3.5 2'],
        ];
  for (const [tag, value] of paths) {
    const node = document.createElementNS(namespace, tag);
    if (tag === 'circle') {
      const [cx, cy, r] = value.split(',');
      node.setAttribute('cx', cx);
      node.setAttribute('cy', cy);
      node.setAttribute('r', r);
    } else {
      node.setAttribute('d', value);
    }
    svg.append(node);
  }
  return svg;
}
