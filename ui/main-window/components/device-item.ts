/**
 * InputDevice 二级页的单个设备条目（radio 风格 + 实时波形），
 * 见 brainstrom/plan.md §6.2。
 */

import type { AudioDeviceInfo } from '../../shared/types';

export interface DeviceItemProps {
  device: AudioDeviceInfo;
  selected: boolean;
  onSelect: (deviceId: string) => void;
}

export function renderDeviceItem(props: DeviceItemProps): HTMLElement {
  throw new Error('Not Implemented');
}
