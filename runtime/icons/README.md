# Logo 图标资源

- `app-icon.svg`：中性灰色主源文件，供安装包和系统静态图标使用。
- `wordmark-d.png`：从主界面艺术字中提取的 `D`，由 SVG 蒙版着色。
- `logo-dark.png` / `logo-light.png`：主窗口运行时图标；深色系统使用白色，浅色系统使用中性灰。
- `tray-dark.png` / `tray-light.png`：对应主题的托盘图标。
- `32x32.png`、`128x128.png`、`128x128@2x.png`、`icon.ico`：Tauri 打包资源。

在仓库根目录运行以下命令可重建全部栅格资源：

```powershell
.\runtime\icons\generate-icons.ps1
```

脚本使用 Tauri 的 SVG 渲染器处理 `stroke-linecap="round"`，不要再为两段外弧手工叠加端点圆帽。
