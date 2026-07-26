/** InputDevice 二级页的单个设备条目。 */

import type { AudioDeviceInfo } from '../../shared/types';

export interface DeviceItemProps {
  device: AudioDeviceInfo;
  selected: boolean;
  onSelect: (deviceId: string) => void;
}

export function renderDeviceItem(props: DeviceItemProps): HTMLElement {
  const item = document.createElement('button');
  item.type = 'button';
  item.className = 'device-item';
  item.setAttribute('role', 'radio');
  item.setAttribute('aria-checked', String(props.selected));
  if (props.selected) {
    item.classList.add('device-item--selected');
  }
  item.addEventListener('click', () => props.onSelect(props.device.id));

  const radio = document.createElement('span');
  radio.className = 'device-item__radio';
  radio.setAttribute('aria-hidden', 'true');

  const copy = document.createElement('span');
  copy.className = 'device-item__copy';

  const name = document.createElement('span');
  name.className = 'device-item__name';
  name.textContent = props.device.name;

  const detail = document.createElement('span');
  detail.className = 'device-item__detail';
  detail.textContent = props.device.isDefault ? '系统默认设备' : '音频输入设备';
  copy.append(name, detail);

  item.append(radio, copy);
  if (props.selected) {
    const meter = document.createElement('span');
    meter.className = 'device-item__meter';
    meter.setAttribute('aria-hidden', 'true');
    for (let index = 0; index < 4; index += 1) {
      const bar = document.createElement('span');
      bar.style.height = '4px';
      meter.append(bar);
    }
    item.append(meter);
  }
  return item;
}
