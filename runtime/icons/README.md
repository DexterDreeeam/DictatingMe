# Logo 图标资源

- `app-icon.svg`：Logo 几何主源文件；生成脚本会替换主题颜色。
- `wordmark-d.png`：从主界面艺术字中提取的 `D`，由 SVG 蒙版着色。
- `logo-dark.png` / `logo-light.png`：主窗口运行时图标；深色系统使用白色，浅色系统使用中性灰。
- `tray-dark.png` / `tray-light.png`：对应主题的托盘图标。
- `32x32.png`、`128x128.png`、`128x128@2x.png`、`icon.ico`：Tauri 打包资源。

Windows 11 任务栏会忽略运行时 `WM_SETICON` 的基础图标并读取 EXE/快捷方式资源，因此静态打包图标使用 Dark 白色版本；托盘与窗口图标仍按系统主题动态选择。

在仓库根目录运行以下命令可重建全部栅格资源：

```powershell
.\runtime\icons\generate-icons.ps1
```

脚本使用 Tauri 的 SVG 渲染器处理 `stroke-linecap="round"`，不要再为两段外弧手工叠加端点圆帽。
