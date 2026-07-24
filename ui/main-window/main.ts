/**
 * MainWindow 入口：初始化 Router，注册四个路由，订阅状态变化更新提示。
 */

import { Router } from './router';
import { HomePage } from './pages/home';
import { InputDevicePage } from './pages/input-device';
import { EvokeWordPage } from './pages/evoke-word';
import { HistoryPage } from './pages/history';

function bootstrap(): void {
  throw new Error('Not Implemented');
}

bootstrap();

// 供内部模块复用，避免未使用告警（占位阶段）
export { Router, HomePage, InputDevicePage, EvokeWordPage, HistoryPage };
