# DictatingMe 设计方案（Brainstorm v1.18）

> 状态：v0.2 已实现并生成 NSIS 安装包。四份 architecture/interface 文档继续作为模块边界和后续重构方向；实际代码保留少量兼容接口以维持既有运行链路。

## 修订记录

**v1.18（v0.2 实现）**：
1. **受管 AppData 已落地**：SQLite、模型、训练资源、Profile、录入样本与 History 使用 `%LOCALAPPDATA%\DictatingMe`；全部资源位于 `assets\`，下载暂存和回收目录为 `assets\.staging` / `assets\.trash`。preset/目录清单编译进 EXE 并在首次运行释放到该目录，Program Files 不保存模型，覆盖安装不删除用户数据。
2. **AssetManager 已实现**：`assets/manifest-cn.json` 是声纹资源、分类训练资源和语音模型的中文名称、分类归属、顺序与下载源唯一配置；它作为明文文件安装到 Program Files，并在每次启动时原子覆盖同步到 `%LOCALAPPDATA%\DictatingMe\manifest-cn.json`，AssetManager 只读取 AppData 文件。`assets/sha.json` 仍内嵌并只保存格式、安装路径、文件大小和 SHA。
   - 语音模型同时配置 ModelScope、hf-mirror 两个国内拆分文件源和三个 GitHub 归档源；`{file}` 模板源按 SHA 清单逐文件下载，归档源仍安全解压，两种来源共用并发测速、总进度、逐文件 SHA 与原子安装。
3. **四种处理器已实现**：文字使用本地汉字→拼音 token；语音匹配使用声学特征 + DTW；声纹验证使用 sherpa CAM++ embedding；分类训练使用用户正样本、MS-SNSD 负样本和 Logistic 分类器。
4. **独立评分系统已实现**：滚动真实麦克风音频，融合语音活动、KWS、模板/声纹/分类分数和灵敏度阈值；KWS 命中后的文字分保持 800ms，再按真实时间衰减，确保 100ms Preview 能稳定越过阈值；Preview 使用非对称 EMA 与 accepted 迟滞，真实唤醒判定仍使用未平滑候选分数。
5. **实际 UI 已接入**：首页使用用户提供并处理为透明白色层次的手写 `Dictating Me` 图片字标（不可选择/拖拽/交互）；语音模型下载/选择、四模式资源门控、5 秒录音、录音锁定、处理 operation、灵敏度百分比与真实评分计量均已接入；下载按钮 Ready 后隐藏，MainWindow 根据页面实际内容以动画伸缩高度。
6. **安装包入口已实现**：根目录 `run_install.cmd` 提权后调用 `assets/run-package.ps1` 构建 per-machine NSIS，构建成功后按精确 Program Files 路径停止旧实例，使用 `/S` 静默覆盖 `C:\Program Files\DictatingMe`，验证安装文件并启动新版；release 使用 Windows GUI subsystem（无控制台），CMD 最后暂停。旧版 LocalAppData 应用安装会在保护用户数据后清理。
   - `run_install_dev.cmd` 使用相同静默覆盖流程构建可安装 Debug NSIS；Windows 的 dev/debug/release 应用统一使用 GUI subsystem，不创建应用 console，DEBUG 级活动只写入 `%LOCALAPPDATA%\DictatingMe\logs\debug`。
   - `run_release.cmd` 只执行资源校验和 Release NSIS 构建，不停止、安装或启动程序；每次清空并导出最新安装包到被 Git 忽略的根目录 `release\`，同时输出并复核 SHA-256。
7. **后端就绪握手已实现**：MainWindow WebView 在隐藏状态等待 Settings/Runtime 命令成功，构建首页后调用 `frontend_ready`；Runtime 只在该命令后创建 Tray 并显示 MainWindow。后端初始化失败时前端窗口不会出现。
8. **安装器快捷方式策略**：保留开始菜单入口；移除 Finish 页 “Create desktop shortcut” 选项，并在 silent/passive 安装后清理桌面快捷方式。

**v1.17（待评审设计）**：
1. **架构文档改为图优先**：Evoke/Storage 的 architecture 和 interface 页面使用模块图、状态图、边界图与时序图，文字只保留约束说明。
2. **新增独立 SettingsHandle**：配置、下载、模型选择、Evoke Setup 全部进入 Settings Actor；不再向 RuntimeHandle 增加设置类方法。
3. **Evoke Setup 与 Runtime 零直连**：设置完成只提交 profile 并递增 storage generation，不发送 profile、路径、进度或回调到 Runtime。
4. **RuntimeDataPort 收敛为 3 个方法**：`subscribe_generation`、`acquire_active_bundle`、`append_history`；RuntimeCore 不持有 ManagedStorage/AssetManager/ContentStorage/子 Store。
5. **RuntimeBundle 一次性交付**：进入 Listening 前一次获取 input device、sensitivity、LoadedEvokeProfile 与 Dictation AssetLease；Listening/Loading/Dictating 不查询 Storage。
6. **SettingsSnapshot 收敛 UI 读取**：UI 初始化或收到 `settings-changed { generation }` 后只重取一个 snapshot；事件不携带完整设置 DTO。

**v1.16（待评审设计）**：
1. **四种唤醒设置模式统一建模**：通用文字、语音匹配、声纹验证、分类训练共用 `EvokeModeProcessor` 与同一套 begin/capture/finish/cancel 会话接口。
2. **录入与处理分离**：Runtime 固定采集 5 秒样本；文字模式 0 条、语音匹配 3 条、声纹验证 3 条、分类训练 6 条；处理成功后才原子替换 active EvokeProfile。
3. **新增受管资源系统**：`AssetManager` 依据 `assets/sha.json` 检查目录、探测来源、下载到 staging、逐文件 SHA-256 验证并原子安装。
4. **扩展 AppData 存储**：使用 `ManagedStorage` 统一管理可下载模型、训练资源、EvokeProfile、录入草稿、History 音频、staging/trash 与引用关系。
5. **增加语音模型门控**：只有一个 DictationModel 时，下载校验完成后自动选择；存在多个模型时仍由用户选择。没有 active DictationModel 或 active EvokeProfile 时，播放按钮和 Runtime `request_background` 都不可进入 Listening。
6. **新增评审文档**：Evoke Setup 与 Storage 分开维护，共四份 architecture/interface HTML；本轮不修改 Runtime/UI 实现。

**v1.15（本次修订）**：
1. **新增内置模型目录**：随安装包分发的初始唤醒模型迁移到 `assets/preset/`，删除原 `assets/evoke/`。
2. **保护多个预置模型**：资源脚本修复单个模型时只清理对应模型包目录，不再清空整个分类目录。
3. **同步运行与打包路径**：开发检查、Runtime 模型解析和 Tauri bundle resources 都从 `assets/preset/` 加载初始唤醒模型。

**v1.14（本次修订）**：
1. **统一顶层资源目录**：模型和验收噪声统一存放在 `assets/`，分别使用 `assets/dictation/`、`assets/preset/` 和 `assets/noise/`。
2. **统一资源准备入口**：PowerShell 脚本迁移至 `assets/download-assets.ps1`；Windows 用户从项目根目录运行 `download-assets.cmd`。
3. **同步运行与打包路径**：开发启动检查、Runtime 资源解析和 Tauri bundle resources 全部改为新的 `assets/` 根目录。

**v1.13（本次修订）**：
1. **误唤醒噪声集扩展到 30 个文件**：覆盖人群/机场广播/咖啡厅、公交/汽车/地铁/车站/道路、空调、复印机、键盘打字、吸尘器和洗衣机。
2. **噪声目录统一为 `assets/noise/`**：旧的散落 WAV 会被迁移或清理；文件名带场景和 train/test 来源，方便验收记录。
3. **同时保留训练与独立测试噪声**：22 个 MS-SNSD `noise_train` 文件 + 8 个 `noise_test` 文件，避免只用单一来源样本验收。

**v1.12**：
1. **统一模型与验收资产准备脚本**：资源准备脚本现位于 `assets/download-assets.ps1`；Windows 用户通过根目录 `download-assets.cmd` 启动，自动绕过本机 PowerShell ExecutionPolicy 限制。
2. **模型改为按必需文件验证**：脚本检查解压后的 ONNX/tokens 文件及最小大小；目录非空但文件缺失/异常时自动清理、重新下载并解压。
3. **噪声验收素材纳入按需下载**：MS-SNSD 16kHz WAV 校验 RIFF/WAVE、采样率和最小大小，缺失或异常时下载到 `assets/noise/`。
4. **验收素材不随应用分发**：`assets/noise/` 只供本地误唤醒验收，保持 gitignore，也不加入 Tauri bundle resources。

**v1.11**：
1. **启动后首先显示 MainWindow**：Runtime 初态从 `Listening` 改为 `Configure`；用户查看/修改配置后点击播放，才进入后台 `Listening`。
2. **移除 HUD 矩形底色**：HUD Window 与 WebView 背景均显式设为 RGBA 全透明，根 HTML 不再声明深色 color-scheme，桌面只保留圆角 HUD 本体。
3. **选定误唤醒验收噪声来源**：优先使用 MS-SNSD 的 16kHz Babble、Cafeteria、Traffic、Metro、CopyMachine、VacuumCleaner WAV；仅作为后续人工验收素材，本轮不下载、不集成、不调整模型。

**v1.10**：
1. **运行提示音改为可试听 WAV 资源**：替代过短、过轻的运行时正弦合成；文件位于 `runtime/assets/sounds/wake.wav` 和 `runtime/assets/sounds/end.wav`，编译时通过 `include_bytes!` 内嵌。
2. **提高可听度但保持短促**：唤醒音约 1040Hz / 90ms，结束音约 620Hz / 100ms，PCM 48kHz/16-bit/mono，带短淡入淡出。

**v1.9**：
1. **增加两个极短运行提示音**：`Listening → Loading` 播放高音短滴；`Dictating → Unloading` 播放低音短滴。
2. **提示音不阻塞状态机**：Runtime 启动时持有输出 mixer，状态转移只追加合成音源；提示音使用极短淡入淡出避免爆音，不需要外部音频资源。
3. **严格绑定状态语义**：Loading 中断不播放结束提示音；只有实际退出 `Dictating` 时才播放结束音。

**v1.8**：
1. **Windows 进程整体提升为管理员权限**：DictatingMe 每次启动请求 UAC，以 High 完整性运行 Runtime、WebView、全局输入监听和文本注入。
2. **覆盖管理员前台窗口**：与管理员应用处于相同完整性级别后，剪贴板 + `Ctrl+V` 可以作用于当前前台焦点控件，全局键鼠监听也可以在该窗口中触发 dismiss。
3. **明确安全取舍**：整体提权扩大了应用权限范围，因此 WebView 仅加载本地/开发源，Tauri 使用受限 CSP；不加载任意远程页面或执行外部脚本。

**v1.7**：
1. **输入电平使用强低音量增益曲线**：选中麦克风的波形不再线性映射 RMS；使用 `0.003` 噪声门和 gamma `0.22`，使约 `0.01 → 0.34`、`0.03 → 0.45`、`0.1 → 0.60`，显著提高普通说话音量的可见度。
2. **标题栏空白区域可拖动窗口**：Home/页名/标题栏背景等非按钮区域可直接拖动 MainWindow；Home、播放和电源按钮保持原点击语义。
3. **Streaming Output 改为剪贴板粘贴**：新增后缀写入系统剪贴板，再发送带应用私有标记的 `Ctrl+V`；不再逐 UTF-16 字符模拟键入。
4. **默认唤醒词改为“你好”**：新配置默认使用“你好”；旧默认“小助手”随配置数据库迁移更新，用户仍可在 EvokeWord 页修改。

**v1.6**：
1. **确立文档优先级**：架构、状态、模块关系和主数据流以已审核的 `architecture.html` 为准；窗口结构、页面内容、交互和视觉以已审核的 `ui-mockup.html` 为准；`plan.md` 负责汇总文字说明与决策日志，不再覆盖两个 HTML 事实来源。
2. **同步架构图的逻辑分组和因果链**：采用 `Streaming Output（Text Diff + Text Injector）`、`History + Config` 的逻辑分组；明确必须先发生 Evoke 检测，再由 State Machine 同时授权 DictationModel 加载和 Audio Ring Buffer 缓冲。
3. **补齐非待机状态打开 MainWindow 的流程**：`Loading` / `Dictating` / `Unloading` 收到打开请求时统一完成清理，再进入 `Configure`；重复或过期的异步事件不改变当前状态。
4. **修正生命周期描述**：Runtime 可由托盘“退出”或 MainWindow 电源按钮终止；播放按钮与 MainWindow 关闭请求只进入后台，不终止进程。
5. **同步当前技术边界**：Rust Runtime 使用本地 ONNX 语音流水线；Windows 音频、全局输入监听和 `SendInput` 保持平台适配层隔离；Tauri command/event 只承担同进程前后端边界。

**v1.5**：
1. **`ui-mockup.html` 成为 UI 视觉唯一事实来源**：MainWindow 按 380px 内容尺度设计，采用 `#1e1f22` / `#2b2d31` / `#313338` 的 Discord 式深色层级、`#3f4248` 边框、`#5865f2` 强调色和紧凑圆角组件；移除蓝色描边的大型 Hero/说明区。
2. **标题和页面内容进一步去冗余**：60px 标题栏在首页左侧留空，二级页显示线性 Home 图标与页名，History 的记录数徽标也只放在标题栏；首页内容仅保留居中的渐变斜体 `Dictating Me` 和三张紧凑导航卡片。
3. **二级页严格按 Mock 收敛**：InputDevice 只显示麦克风行，实时电平只出现在选中行；EvokeWord 只显示唤醒词输入与灵敏度滑杆并自动保存；History 只显示可滚动记录卡片列表。
4. **Configure 行为仅属于 Runtime 内部机制**：MainWindow 打开仍会使 Runtime 进入 `Configure`，但 UI 不再展示“配置模式 · 唤醒监听已暂停”横幅，也不通过全局 toast/状态切换文案暴露 `Configure` 或其他 Runtime 状态。进入后台与退出保持透明，仅局部操作失败可就地提示。
5. **HUD 文案固定为“待唤醒”与“听写中”**；控件图标优先使用与 Mock 一致的单色线性 SVG。

**v1.4**：
1. **MainWindow 标题栏由"关闭"改为"播放 + 电源"两个按钮**：右上角不再是单一关闭按钮，改为 ▶ 播放（进入后台运行）与 ⏻ 电源（退出程序）两个独立图标。播放按钮语义与旧"关闭"完全相同（驱动 `MainWindowClosed` 事件，回到 `Listening`/待唤醒，HudWindow 显示）；电源按钮是新增能力——**直接终止整个 Runtime 进程、无需二次确认**，不经过 State Machine，与系统托盘"退出"走同一路径（此前退出**只能**通过系统托盘菜单，本次新增了从 MainWindow 直接退出的入口）。
2. **新增 Tauri 命令**：`request_background`（播放按钮）、`quit_app`（电源按钮），对应前端 `ui/shared/api.ts` 新增 `requestBackground()` / `quitApp()`；新增 `ui/main-window/components/titlebar.ts` 接口。

**v1.3**：
1. **HudWindow "可唤醒" 状态更名为"待唤醒"**：中文界面用语从"监听中"改为"待唤醒"，更准确地表达"尚未被唤醒、等待用户说出唤醒词"的语义，避免与"正在监听/录音"混淆。
2. **MainWindow 采用无边框自定义标题栏**：不使用 OS 原生窗口装饰（无红黄绿/最小化按钮），仅左上角显示应用名"Dictating Me"、右上角一个自制关闭按钮；对应 Tauri 配置 `decorations` 由 `true` 改为 `false`。

**v1.2**：
1. **状态颜色总语义明确**：黄=可被唤醒，绿=正在记录用户输入，灰=无响应状态。据此 `Loading` 状态改为**绿灯**（此前误定为黄灯/过渡态——Loading 期间 Audio Ring Buffer 已在录音，属于"记录输入"而非单纯待机）。
2. **模块图 State Machine 节点加宽**：核心节点大幅加宽，让周边模块可就近直连，减少连接线的长距离绕行与交叉。

**v1.1**：
1. **MainWindow 与 HudWindow 互斥显示**：二者由 Runtime 统一控制，同一时刻有且只有一个可见（此前版本认为 HudWindow 常驻、MainWindow 是访客窗口，本次修正为互斥关系）。
2. **打开 MainWindow 即进入 Configure**：不再区分"首页浏览"与"二级页编辑"——只要 MainWindow 被打开（哪怕只是看首页或 History），Runtime 立即进入 Configure 状态；若当前处于 Loading/Dictating/Unloading，会先自动流转完 Unloading→Listening，再进入 Configure，无需用户二次操作（此前版本认为仅二级编辑页触发锁定，本次推翻）。
3. **DictationModel 的触发因果更明确**：DictationModel 严格只在 EvokeModel 检测到唤醒词、State Machine 收到该事件之后才会被触发加载，模块图对这条因果链做了强化。
4. Configure ↔ Listening 修正为双向转移（打开/关闭 MainWindow 对应两个方向）。
5. 时序图配色按"是否在处理用户声音"重新划分：黄=唤醒词识别阶段、绿=声音采集/转换阶段、灰=不检测声音阶段。

---

## 1. 项目概述

**DictatingMe（DM）** 是一款常驻后台的语音听写工具，交互模式类似"智能音箱"：

1. 软件持续监听麦克风，等待用户说出**唤醒词**；
2. 唤醒后，软件开始**流式**将用户语音转换为文字，并通过剪贴板 + `Ctrl+V` 把新增内容实时粘贴到当前屏幕焦点所在的输入框；
3. 用户做出任意键盘/鼠标操作即视为结束本次听写（dismiss），软件自动收尾并回到监听状态。

核心设计原则：
- **轻量常驻**：唤醒词检测模型（EvokeModel）极小（目标 <20MB），可以长期占用内存运行；听写模型（DictationModel）体积较大，只在唤醒后按需加载，用完即卸载。
- **无感衔接**：从唤醒的一刻起就开始录音缓冲，模型加载完成后无缝处理，不丢字。
- **简单直接**：v1 阶段刻意避免过度设计（不做纠错回退、不做敏感控件保护、不做错误恢复状态机），优先把主链路做扎实。

---

## 2. 技术栈选型

| 层 | 选型 | 说明 |
| --- | --- | --- |
| Runtime（核心） | **Rust + 本地 ONNX 语音流水线** | 长期驻留、状态机调度、模型生命周期、音频与系统输入均在 Rust 内完成 |
| UI | **Tauri + TypeScript** | Runtime 与两个 WebView 窗口同进程；UI 通过 Tauri command/event 读取数据和接收状态 |
| 目标平台 | **Windows 优先（管理员权限）** | v1 整体以 High 完整性运行，以支持管理员前台窗口中的粘贴与 dismiss；启动时显示 UAC |

**架构含义**：因为 Runtime 和 UI 在同一个 Tauri 进程内，模块设计图里的"进程边界"其实只有一条——DM 进程 vs 操作系统。UI（MainWindow / HudWindow）是这个进程内的 WebView 窗口，通过 Tauri 的事件系统订阅 Runtime 状态变化。

---

## 3. 核心概念

### 3.1 Model、Asset 与 EvokeProfile

| 对象 | 用途 | 体积/加载策略 | 技术路线 |
| --- | --- | --- | --- |
| **Preset EvokeModel** | 基础唤醒词检测 | `assets/preset/` 随包发布，常驻内存 | sherpa-onnx KWS；所有模式都复用基础候选检测 |
| **EvokeProfile** | 当前用户唤醒配置 | AppData 小型 manifest + artifact | 文字参数、语音模板、声纹 centroid 或分类器；同一时间只有一个 active profile |
| **Optional Evoke Assets** | 声纹/分类处理依赖 | 首次使用按需下载 | speaker embedding、keyword embedding、negative embedding bank |
| **DictationModel Asset** | 流式语音转文字 | 体积较大，下载后选择，唤醒后加载/结束后卸载 | 当前模型显示名 `sherpa-zipformer-zh-en`，本地 ONNX、离线可用 |

运行时仍然互斥：EvokePipeline 只在 `Listening` 处理麦克风音频；一旦进入 `Loading`/`Dictating`/`Unloading`，唤醒检测停止。设置/下载/录入只在 `Configure` 内运行，不新增 Runtime 状态。

### 3.2 两个 Window（互斥显示）

| Window | 定位 | 显示条件 |
| --- | --- | --- |
| **MainWindow** | 软件的配置/设计界面（输入设备 / 唤醒词 / 语音模型 / 历史记录四个入口 + 设置子页） | 仅在 `Configure` 状态显示 |
| **HudWindow** | 运行时的状态浮层（Overlay） | 在 `Listening` / `Loading` / `Dictating` / `Unloading` 状态显示（无边框、半透明、不可交互） |

**MainWindow 与 HudWindow 由 Runtime 统一控制、互斥显示**：同一时刻有且只有一个可见。触发关系是**"窗口显示由状态决定"而非"窗口决定状态"的单向依赖**，但入口动作（点击托盘图标打开 MainWindow）会反过来驱动状态切换：

- **打开 MainWindow** → Runtime 立即（或自动流转后）进入 `Configure`，同时隐藏 HudWindow
- **关闭 MainWindow**（返回/退出配置）→ Runtime 回到 `Listening`，同时显示 HudWindow

详见第 4 节状态机中 `Configure` 的双向转移。

### 3.3 Runtime

Runtime 是 DM 的核心，承担：
- 状态机驱动（见第 4 节）
- 系统托盘的创建与持有（图标跟随系统明暗主题，但不随听写状态改变；左键点击唤出 MainWindow，右键菜单提供"退出"）
- 运行期音频采集、模型加载/卸载、文字注入、全局键鼠监听和 History append

配置、资源下载、模型选择和 Evoke Setup 属于独立 **Settings Plane**，由 `SettingsHandle + SettingsCoordinator + ManagedStorage` 管理；Runtime 只通过 `RuntimeDataPort` 获取运行 bundle，不直接访问任何 Store。

**关键生命周期事实**：Runtime 与 SystemTray 共生共灭；MainWindow / HudWindow 都是 Runtime 派生的"访客窗口"，二者互斥显示，均可被显示/隐藏，不影响 Runtime 主循环本身。软件**不做开机自启动**；通过托盘菜单“退出”或 MainWindow 电源按钮才终止整个 Runtime 进程。MainWindow 播放按钮、窗口关闭请求都只进入后台运行。

**权限事实**：Windows UIPI 不允许 Medium 完整性进程向 High 完整性窗口发送 `Ctrl+V`，也不允许低完整性钩子完整观察高完整性窗口输入。因此 v1 的整个 DictatingMe 进程通过 `requireAdministrator` manifest 和开发启动脚本提升到 High 完整性。

---

## 4. Runtime 状态机

### 4.1 状态定义

**颜色语义**：黄 = 可被唤醒（EvokeModel 监听中）；绿 = 正在记录用户输入（录音缓冲或流式转写）；灰 = 无响应状态（不监听也不记录）。

| 状态 | 说明 | EvokeModel | DictationModel | 显示窗口 |
| --- | --- | --- | --- | --- |
| **Configure** | MainWindow 已打开（首页四卡片、设置向导或任意二级页），全局锁定 | 停止 | 不适用 | MainWindow（灰） |
| **Listening** | 进入后台后的待机状态，等待唤醒词 | 运行中 | 未加载 | HudWindow（黄灯） |
| **Loading** | 唤醒词已触发，DictationModel 正在加载；**同时立即开始录音并缓冲** | 停止 | 加载中 | HudWindow（**绿灯**，录音已开始，视为记录输入中） |
| **Dictating** | 模型加载完成，流式转写进行中；缓冲区音频无缝作为第一批输入，之后接实时流 | 停止 | 运行中 | HudWindow（绿灯） |
| **Unloading** | 用户 dismiss 或“打开 MainWindow”请求触发的收尾状态：停止转写、丢弃未转换内容、卸载模型、完成 History 写入 | 停止 | 卸载中 | HudWindow（灰，不再记录） |

**MainWindow 与 HudWindow 严格互斥**：`Configure` 显示 MainWindow、隐藏 HudWindow；其余四个状态显示 HudWindow、隐藏 MainWindow。

### 4.2 状态转移

```
[*] --> Configure

Listening --> Configure   : 打开 MainWindow
Configure --> Listening   : 关闭 MainWindow

Listening --> Loading     : EvokeModel 检测到唤醒词
Loading  --> Dictating    : DictationModel 加载完成（缓冲音频无缝喂入）
Loading  --> Unloading    : 打开 MainWindow 请求（中断加载，走统一清理出口）

Dictating --> Unloading   : 用户 dismiss（任意键盘键 / 鼠标左右中侧键）或"打开 MainWindow"请求
Unloading --> Listening   : 清理完成（卸载模型、丢弃未转换内容、写入 History）
```

**要点**：
- Runtime 启动后直接进入 `Configure` 并显示 MainWindow；点击播放或关闭 MainWindow 后才进入 `Listening`。
- `Configure` 只能从 `Listening` 直接进入/退出（**双向转移**）；触发条件是"打开/关闭 MainWindow"这个动作本身，不再区分首页浏览还是二级页编辑——只要 MainWindow 处于打开状态，Runtime 就处于 `Configure`。
- 若用户在 `Loading`、`Dictating` 或 `Unloading` 期间发出“打开 MainWindow”请求，Runtime 会记住该请求，**自动、连续地**完成 `Unloading` 清理，再进入 `Configure`，全程无需用户二次操作。
- 异步模型加载和清理事件携带会话标识；已经过期的完成事件、重复 dismiss 和重复窗口请求不会复活旧会话，也不会改变已经稳定的状态。
- `Dictating` **没有静音超时**机制——只要用户不做键鼠操作、也没有打开 MainWindow，会一直保持监听转写，哪怕中间沉默很久。这是刻意的设计取舍，兼容"说话中间停顿思考"的场景。
- `Loading` 阶段录制的音频是 `Dictating` 处理的**第一批数据**，两者无缝衔接，不丢帧。
- 详细图示见 `architecture.html`（状态机图 + 时序图 tab）。

---

## 5. 模块设计

Runtime（Rust 进程）内部划分为以下模块，详见 `architecture.html` 的"模块设计图"：

| 模块 | 职责 |
| --- | --- |
| **State Machine** | 驱动 4.1 节的 5 个状态及其转移，是整个 Runtime 的调度中枢 |
| **Audio Capture** | 麦克风设备管理与持续采集；Listening 音频进入 EvokeModel，Configure 音频用于选中设备的实时电平 |
| **Audio Ring Buffer** | Loading 阶段的录音缓冲区，保证进入 Dictating 时无缝喂入 |
| **EvokePipeline** | 读取 active EvokeProfile，组合基础 KWS 与模式 verifier；仅在 Listening 激活，输出统一 `EvokeDecision` |
| **EvokeSetupService** | Configure 内管理 begin/capture/finish/cancel，会从 Processor Registry 选择四种模式共用 trait 的实现 |
| **DictationModel Engine** | 通过 active DictationModel asset lease 加载本地 ONNX；**仅由 State Machine 在收到 EvokePipeline 接受结果后触发加载** |
| **Streaming Output** | 由 Text Diff + Text Injector 组成：比较完整识别假设，只把新增后缀写入剪贴板并通过 `Ctrl+V` 粘贴到当前焦点，不做回退 |
| **Global Input Monitor** | 全局监听键盘/鼠标事件，用于识别 dismiss 信号 |
| **SettingsHandle / SettingsCoordinator** | MainWindow 设置命令的独立 Actor；串行执行 mutation，生成 SettingsSnapshot/readiness；不经过 RuntimeHandle |
| **ManagedStorage** | Settings Plane 内组合 Database、ContentStorage、AssetManager、Profile/Setup/History/Config Store；RuntimeCore 不直接持有 |
| **AssetManager** | 解释 `assets/sha.json`，执行目录检查、来源探测、下载、解压、SHA 验证、版本 lease 与无引用旧资产清理 |
| **RuntimeDataPort** | Runtime 与 Storage 的唯一边界：generation watch、一次性 RuntimeBundle、History append |
| **Tray Manager** | 创建/持有系统托盘图标（跟随系统明暗主题，不随听写状态改变）；左键点击 = 向 State Machine 发出"打开 MainWindow 请求"，右键菜单提供"退出" |
| **Window Manager** | 基于 Tauri 多窗口能力，根据 State Machine 广播的状态**互斥显示** MainWindow 或 HudWindow（同一时刻只有一个可见） |

### 5.1 数据流（一次完整唤醒—听写—收尾周期）

1. `Audio Capture` 持续采集麦克风音频；在 `Listening` 中，音频送入 `EvokeModel Engine`
2. `EvokeModel Engine` 检测到唤醒词 → 通知 `State Machine`；`Listening → Loading` 被接受时立即播放唤醒短滴；**只有此后** State Machine 才同时授权 `DictationModel Engine` 异步加载和 `Audio Ring Buffer` 开始缓冲，二者都不会提前发生
3. 模型加载完成 → `State Machine` 切到 `Dictating`：`Audio Ring Buffer` 中的缓冲内容优先喂给 `DictationModel Engine`，随后接实时音频流
4. `DictationModel Engine` 持续产出当前完整识别假设 → `Streaming Output` 中的 Text Diff 计算新增后缀 → Text Injector 模拟打字到当前焦点控件
5. `Global Input Monitor` 检测到任意键鼠事件，**或 `Tray Manager` 收到"打开 MainWindow"请求** → `State Machine` 从 `Dictating` 切到 `Unloading` 时播放结束短滴，然后停止喂音频、丢弃未转换内容、卸载 `DictationModel Engine`
6. 收尾完成 → 本次听写的最终文本 + 录音 + 时间戳写入 `History Store` → `State Machine` 回到 `Listening`（若收尾原因是"打开 MainWindow 请求"，则紧接着自动进入 `Configure`，`Window Manager` 随即切换显示 MainWindow、隐藏 HudWindow）

（完整时序图见 `architecture.html`）

### 5.2 唤醒设置数据流

1. `EvokeSetupService.begin(mode, phrase)` 检查模式依赖并返回统一 `EnrollmentPlan`。
2. Runtime 使用当前输入设备固定采集 5 秒，复用现有 `input-level` event 驱动音量条；文字/语音/声纹/分类分别需要 0/3/3/6 条。
3. `finish` 创建通用 processing operation，并由对应 `EvokeModeProcessor` 处理用户录音；Processor 只通过 Storage/Asset port 访问逻辑引用。
4. 处理成功后原子提交 profile/artifact/asset 引用并更新 `active_evoke_profile_id`；失败或取消保留旧 active profile。
5. 详细业务架构和接口见 `architecture-evoke-setup.html`、`interface-evoke-setup.html`。

### 5.3 资源下载与存储数据流

1. DownloadButton 使用 `asset_link_list + asset_path` 初始化；前端按 `asset_path` 复用应用级下载控制器，页面切换不丢失 operationId/phase/progress，新页面实例先恢复进行中样式，非下载状态重新调用 `AssetManager.inspect`；后端按 asset ID 去重进行中的安装 operation。
2. 缺失或校验失败时，AssetManager 对 manifest 中全部可信来源并发发送 256KiB Range 探测，统一等待最多 12 秒，排除少于 64KiB/HTTP 失败/超时响应并按包含首包延迟的实际吞吐排序；随后从最快源开始完整下载，启动失败才按测速排名回退。下载进入 `%LOCALAPPDATA%\DictatingMe\assets\.staging\<operation>\download.part`，安全解压并完成 size/SHA-256 验证后立即删除压缩文件，再原子安装到 `assets\` 子目录。
3. 模型、训练资源、录入草稿、profile artifact 与 History 音频统一使用 ContentRef/AssetLease，业务模块不直接删除绝对路径。
4. ManagedStorage 通过 reference、lease、保留期、LRU、staging/trash 和启动 reconcile 维护旧内容。
5. 详细存储架构和接口见 `architecture-storage.html`、`interface-storage.html`。

### 5.4 Settings / Runtime 最小边界

1. Evoke Setup、资源下载和模型选择不调用 RuntimeHandle；成功 mutation 只递增 `generation`。
2. RuntimeCore 启动时订阅 `watch<u64>`；generation 变化只把 `bundle_dirty` 设为 true，不立即加载模型。
3. 用户点击播放、准备从 Configure 进入 Listening 时，如果 dirty，Runtime 只调用一次 `acquire_active_bundle()`。
4. `RuntimeBundle` 自包含 input device、sensitivity、LoadedEvokeProfile、Dictation AssetLease；运行期不再访问 Storage。
5. Unloading 收尾时 Runtime 只调用一次 `append_history()`；这是运行期唯一 Storage 写入。
6. `RuntimeDataPort` 固定为三个方法，不扩展成通用 Store/Asset/Profile 查询接口。

---

## 6. MainWindow UX 设计

**视觉事实来源**：以最新 `ui-mockup.html` 为准；本节文字只描述行为，不覆盖 Mock 的结构、尺度和视觉规范。

**整体风格**：围绕 380px 内容宽度的 Discord 式深色极简界面。背景 `#1e1f22`，面板 `#2b2d31` / `#313338`，边框 `#3f4248`，强调色 `#5865f2`，次要文字 `#949ba4`。

**窗口装饰**：不使用 OS 原生标题栏（`decorations: false`），使用 60px 自定义标题栏。首页左侧留空；二级页左侧同行显示线性 Home 图标和页名，History 额外显示记录数；右侧保留两个紧凑线性图标按钮，不提供最小化按钮：
- **▶ 播放**：进入后台运行——隐藏 MainWindow，Runtime 回到 `Listening`（待唤醒），HudWindow 随之显示；语义与"关闭窗口"完全相同，只是不再用"关闭/✕"的说法，强调"程序仍在运行、只是转入后台"。
- **Readiness 门控**：没有已选择且校验通过的语音模型，或没有可用 active EvokeProfile 时，播放按钮变灰且不可点击；Runtime 的 `request_background` 仍必须重复校验，不能只依赖 UI。
- **⏻ 电源**：退出程序——**无需二次确认**，直接终止整个 Runtime 进程；与系统托盘右键菜单"退出"是同一操作的另一入口（此前退出只能通过系统托盘）。

**导航结构**：首页四张卡片（输入设备 / 唤醒词 / 语音模型 / 历史记录），单行布局：线性 SVG 图标 + 固定标题在左，当前值右对齐显示在同一行，不用 chevron 箭头；点击进入对应二级页面，标题栏的 Home 图标回到首页。

**Runtime 内部 Configure 行为**：MainWindow **只要处于打开状态**（无论停留在首页还是任意二级页），Runtime 即处于 `Configure`，EvokeModel 停止、HudWindow 隐藏；这是内部窗口/状态协调机制，**不在 MainWindow 中通过横幅、toast 或状态切换提示暴露**。用户点击播放进入后台后恢复监听。

**窗口拖动**：60px 标题栏的所有非按钮区域都可拖动 MainWindow；Home、播放和电源按钮不触发拖动。

### 6.1 首页

- 内容区只显示居中的渐变斜体 `Dictating Me` 标题和四张纵向紧凑卡片。
- 卡片标签固定为**输入设备 / 唤醒词 / 语音模型 / 历史记录**；右侧分别显示当前设备、当前唤醒词、已选模型名、`18 / 20 条记录`。
- 没有选中语音模型时，“语音模型”卡片使用暗红色并显示“需要设置”，同时播放按钮不可用。
- 页面无说明段落、Configure 横幅或其他状态提示。

### 6.2 InputDevice 二级页

- 正文只列出系统所有可用麦克风设备的紧凑单选行，不重复页面标题或说明；始终按“有效持久化设备 → 系统默认 → 第一项”恰好选中一个，整行单击一次立即切换，已选项不可通过重复点击取消。
- 选中行使用强调色边框/淡色背景，且只有选中行显示由真实输入电平事件驱动的波形条。
- 波形使用 `0.003` 噪声门和 gamma `0.22` 的强低音量增益曲线，目标映射约为 `0.01 → 0.34`、`0.03 → 0.45`、`0.1 → 0.60`，高音量区逐渐压缩。

### 6.3 EvokeWord 二级页

- 主页面显示当前唤醒词、带百分比的灵敏度滑杆、只读识别检测计量条、当前输入设备名和“设置唤醒词”按钮；计量条、阈值线和数字以逐帧插值连续追踪平滑 Preview，不显示 10 Hz 事件阶跃。
- 设置向导第一步为四选一：通用文字、语音匹配、声纹验证、分类训练；声纹/分类依赖资源未 Ready 时不可选择，但其 DownloadButton 可点击。
- DownloadButton 在连接/验证阶段显示持续旋转 spinner，下载进度在 Home 往返后继续显示，Ready 后直接隐藏；MainWindow 在页面、操作状态和内容高度变化时平滑调整窗口高度。
- 具体设置页统一包含文字输入；语音匹配/声纹验证各录 3 次，分类训练录 6 次，每次由 Runtime 固定采集 5 秒。
- 录制期间按钮显示精确到 0.1 秒的倒计时，Home/播放/返回都禁用并变灰，仅电源按钮可用；录制结束后恢复。
- 录制进度使用小格表达：已完成为绿色、当前录制为明显高亮、下一条待录制为轻微高亮。
- 完成最后一次录制后出现“完成设置”；只有分类训练显示预计处理时间。处理成功后原子替换当前 active EvokeProfile。
- 处理并替换 active EvokeProfile 期间继续保持全局操作锁：Home、播放和返回不可用，仅电源按钮保留。
- 通用文字无录音；新配置默认文字为“你好”。灵敏度仍单独防抖持久化。

### 6.4 SpeechModel 二级页

- 语音模型列表与唤醒模式卡片使用相同选择视觉；当前模型显示名为 `sherpa-zipformer-zh-en`，副标题为“中文, English”。
- 模型未下载或 SHA 不正确时没有默认选中项，模型卡不可选择；DownloadButton 依次显示下载、连接中、进度、验证中、就绪。
- 下载完成只把模型变为可选择，用户仍需主动选中；选中后首页卡恢复正常，播放按钮才可能解锁。

### 6.5 History 二级页

- 正文仅为最近最多 20 条记录的可滚动紧凑卡片列表（FIFO，第 21 条进来自动淘汰最旧的 1 条），记录数只显示在标题栏。
- 每条记录顶行包含时间戳与线性播放/复制按钮，文本显示在下方。
- 支持**复制**文本；**不支持删除、不支持搜索**（v1 范围明确排除）
- 播放/复制成功不显示 toast；失败可在列表附近就地提示。

---

## 7. HudWindow UX 设计

- **形态**：主屏宽、64px 高的全透明点击穿透层，中心只显示 126×30px 紧凑胶囊；胶囊为 50% 黑底、10px 圆角，透明层用于容纳不会被 WebView 裁切的左右绿色光束
- **位置**：固定在**主屏幕**顶部居中，距离屏幕顶部约 **80px**；不跟随鼠标，多屏时只在主屏幕显示
- **与 MainWindow 互斥**：只要 MainWindow 打开（`Configure` 态），HudWindow 立即隐藏；MainWindow 关闭回到 `Listening` 及之后的状态时，HudWindow 重新出现
- **光束进入**：`Listening → Loading/Dictating` 时，左右各从 4px 中心短光束开始，160ms 淡入、240ms 扩展到合计屏宽 1/4，之后由真实 `input-level` 接管
- **光束视觉**：胶囊在所有状态都不使用任何向外扩散光晕；绿色圆点和左右光束均为纯色亮芯，不使用 drop-shadow，避免形成大面积荧光
- **音量响应**：输入电平使用 gamma `0.38`；增大立即追随，降低保持峰值 180ms 后按约 520ms 时间常数缓释
- **光束退出**：`Dictating/Loading → Unloading/Listening` 从当前长度按约 165ms 收缩；每侧低于屏宽 5.5% 后保持短长度并用 320ms smoothstep 淡出。`Configure → Listening` 首次显示时从左右合计屏宽 1/2 的余光开始收缩
- **状态映射**（黄=可唤醒，绿=记录输入中，灰/隐藏=无响应）：
  - `Listening` → 黄灯
  - `Loading` → 绿灯（录音缓冲已开始，等同于"正在记录输入"）
  - `Dictating` → 绿灯
  - `Configure` → HudWindow 不存在/隐藏（MainWindow 取而代之）
  - `Unloading` → 胶囊切黄并开始绿色光束收缩/淡出，完成后保持待唤醒视觉
- **用户文案**：`Listening` 固定显示“待唤醒”；`Loading` / `Dictating` 统一显示“听写中”，不向用户区分模型加载阶段。
- **托盘图标**：跟随系统明暗主题切换白色/中性灰版本，但不随听写状态变化；左键点击 = 打开 MainWindow（触发 `Configure`）；所有运行时状态提示职责都交给 HUD

---

## 8. 关键机制细节

### 8.1 流式文字注入（剪贴板粘贴、无回退修正）

- `DictationModel` 每次产出的是"当前完整识别文本"（而非单纯 delta），`Text Diff Engine` 对比上一次完整文本，只取**新增的后缀部分**
- 每个新增后缀写入系统剪贴板，再通过 Windows `SendInput` 发送 `Ctrl+V` 到当前焦点；剪贴板内容会更新为最近一次注入文本
- `Ctrl+V` 输入携带应用私有标记，Global Input Monitor 不会把 DM 自己的粘贴快捷键识别为 dismiss
- **不做修正回退**：如果模型后续修正了之前的识别结果（流式 ASR 常见现象），DM 不会退格重打，只会继续追加新内容——这是刻意的简化取舍，避免在任意应用中做"选中删除重打"带来的光标/焦点风险
- **无条件发送到当前操作系统焦点控件**，不做密码框等敏感控件的识别与保护（v1 明确排除，见 Future Work）
- 其他真实或外部注入的键鼠事件仍按 dismiss 处理

### 8.2 Loading → Dictating 的无缝衔接

- 唤醒词触发的瞬间即开始录音，写入 `Audio Ring Buffer`
- `DictationModel` 加载完成后，**先处理缓冲区里的音频**，再无缝衔接实时流，确保"从喊出唤醒词那一刻起"的语音都不丢失
- 这也是"Dictating 状态的 input 从 trigger 后开始算，而不是从模型加载完成后开始算"这一需求的具体实现方式

### 8.3 Dismiss 判定

- **任意**键盘按键、鼠标左键/右键/中键/侧键，只要检测到一次，立即视为 dismiss
- 需要**全局**监听（不局限于 DM 自身窗口获得焦点的情况），因为用户听写时通常正在别的应用里输入
- Dismiss 触发后：立即停止音频喂入、**丢弃尚未转换完成的内容**（不会等模型处理完最后一段再收尾）、卸载 `DictationModel`
- **"打开 MainWindow"请求与 dismiss 等价**：点击托盘图标本身就是一次鼠标操作，天然会被 Global Input Monitor 捕获为 dismiss；因此 `Loading`/`Dictating` 期间点击托盘图标，会先走完 `Unloading → Listening` 的收尾流程，再自动进入 `Configure`

### 8.4 Configure 触发粒度

- **MainWindow 打开即 `Configure`**：不区分首页、InputDevice、EvokeWord、History 中的任何页面，只要 MainWindow 处于打开状态，Runtime 就处于 `Configure`，EvokeModel 完全停止
- 若打开请求发生在 `Loading`/`Dictating`/`Unloading`，会先自动流转完 `Unloading → Listening`，再进入 `Configure`（见 8.3）

### 8.5 运行提示音

- **唤醒提示音**：`Listening → Loading`，`runtime/assets/sounds/wake.wav`，约 1040Hz / 90ms
- **结束提示音**：`Dictating → Unloading`，`runtime/assets/sounds/end.wav`，约 620Hz / 100ms
- 两个 PCM WAV 在编译时内嵌，由 Runtime 解码后加入常驻输出 mixer；提示音不可用只记录日志，不阻断听写状态
- `Loading → Unloading` 的中断不播放结束音，避免把“尚未开始正式转写”误提示为一次完整听写结束

---

## 9. 数据持久化

大文件统一位于 `%LOCALAPPDATA%\DictatingMe`，SQLite 只保存 metadata、状态和引用。内嵌 preset 首次启动释放到 `assets\preset`；可下载内容进入 `assets\.staging\<operation>`，验证后删除压缩包并原子安装，失败立即清理该 operation 目录。

| 数据 | 存储内容 | 策略 |
| --- | --- | --- |
| manifest-cn.json | 三类中文资源列表、name、primary、sources | Program Files 明文源 → 启动时覆盖 AppData 明文副本 → Runtime 读取；所有非 bundled SHA 条目必须恰好出现一次 |
| sha.json | kind、version、format、installPath、逐文件 size/SHA | 只负责技术安装与完整性，不保存本地化名称或 URL |
| Config | 当前输入设备、敏感度、active EvokeProfile ID、active DictationModel asset ID | 只保存选择 ID，不复制 profile/asset 内容 |
| History | 音频 ContentRef + 转写文本 + 时间戳 | FIFO 最多 20 条；淘汰时释放引用，由 ContentStorage 决定是否删除物理文件 |
| Asset Installation | 模型/训练资源版本、目录、逐文件 SHA、最近使用时间 | Catalog 校验、原子安装；active/leased 版本不可清理 |
| EvokeProfile | 模式、文字、preset/asset 引用、派生 artifact | 单一 active；新 profile 成功提交后原子替换，旧 profile 保留一个 7 天回滚窗口 |
| EvokeSetup | 草稿阶段、5 秒录音引用、质量结果、operation ID | 成功后释放原始录音；失败/取消草稿最多保留 24 小时用于恢复 |
| Operation | 下载/验证/处理/修复的阶段、进度、错误 | 下载与训练共用；崩溃后标记 Interrupted 并由 reconcile 清理或恢复 |
| Staging / Trash | 未提交下载、待删除文件 | 下载成功或失败均立即删除本次 Staging；旧版本先原子移动到 Trash 后清理 |

**清理不变量**：active profile、active dictation asset、任何有 lease/reference 的内容都不能自动删除。旧 asset version 只有在引用数为 0 且超过保留期后才按 LRU 清理。详见 `architecture-storage.html`。

---

## 10. 未来路线图（Future Work，v1 明确不做）

1. **错误处理状态**：麦克风占用/拔出、DictationModel 加载失败、网络/资源异常等场景的独立 `Error` 状态与恢复流程
2. **敏感控件保护**：识别密码框等场景并暂停/跳过文字注入
3. **多唤醒词支持**：同时启用多个唤醒词，并可分别绑定不同行为/配置（v1 只支持单一生效唤醒词）
4. **多 Profile 管理/回滚 UI**：底层保留 retired profile 能力，但首版只展示单一 active profile
5. **跨平台扩展**：macOS / Linux 的音频采集、全局键鼠监听、模拟输入、托盘能力实现（Rust 侧已预留 trait 抽象）
6. **云端 ASR 可切换**：DictationModel 除本地 ONNX 外，预留切换到 DashScope 云端流式 API 的能力（更好效果 vs 离线可用之间的取舍开关）
7. **AI 文本润色**：转写结果的标点恢复、口语化去除、AI 二次润色等（当前范围只做"忠实转写 + 追加打字"）

---

## 11. 已知风险与开放问题

| # | 风险/问题 | 说明 |
| --- | --- | --- |
| 1 | 本地 ONNX ASR 的推理性能 | 本地部署对 CPU/内存有一定要求，需要持续实测模型加载耗时、实时率和流式转写延迟 |
| 2 | `Ctrl+V` 在部分应用中可能被拦截 | 剪贴板写入不等于跨权限输入；普通权限进程向管理员窗口发送 `Ctrl+V` 仍可能被 UIPI 拦截，部分游戏/安全软件也可能拦截 |
| 3 | 全局键鼠监听的实现方式与权限 | Windows 上通常用低级钩子（`SetWindowsHookEx`）或 Raw Input；需评估杀毒软件/安全软件的误报风险 |
| 4 | `Unloading` 的 HUD 过渡视觉 | 颜色语义已明确（黄=可唤醒/绿=记录输入/灰=无响应），Loading 已定为绿灯；`Unloading` 具体是瞬时熄灭还是有渐隐动画，细节留待下一轮 UI 细化 |
| 5 | 流式识别"仅追加不回退"的体验影响 | 需要在实际使用中验证：千问 ASR 流式输出的 partial 结果修正频率是否会导致明显的"错字堆积"现象 |
| 6 | 整体管理员权限的安全面 | Runtime 与 WebView 都以 High 完整性运行；必须保持本地内容、受限 CSP 和最小 Tauri capability，禁止加载不可信远程页面 |
| 7 | 下载接口的任意路径/来源风险 | Renderer 的 assetPath/link list 不可信；Runtime 必须限制在 AppData managed root，并与 `assets/sha.json` 的可信来源做交集 |
| 8 | 训练资源与磁盘膨胀 | 分类资源、旧模型版本和草稿录音可能持续占用磁盘；必须依赖引用计数、lease、保留期和 LRU 清理，不允许直接按目录猜测删除 |
| 9 | 少样本个性化准确率 | 3/3/6 条录音只适合模板、声纹 centroid 或轻量分类头；必须在真实噪声和相似发音上评估误接受/误拒绝 |

---

## 12. 需求确认记录（决策日志）

本方案基于持续的头脑风暴澄清问答逐步确定，供后续回溯设计依据（v1.0 首轮共 17 条，#18 起为后续修订轮次新增）：

| # | 主题 | 结论 |
| --- | --- | --- |
| 1 | 技术栈 | Runtime: Rust；UI: Tauri（同进程） |
| 2 | DictationModel 部署 | 本地 ONNX 模型（非云端 API），离线可用 |
| 3 | 流式输出机制 | ~~逐字符模拟键盘输入且不用剪贴板~~ → **v1.7 修正**：剪贴板写入 + `Ctrl+V`，仍保持纯追加、不做修正回退（见 #40） |
| 4 | Dictating 结束条件 | 无静音超时，仅靠任意键鼠 dismiss |
| 5 | 目标平台 | Windows 优先，架构预留 macOS/Linux |
| 6 | Configure 触发时机 | ~~仅二级编辑页（InputDevice/EvokeWord）触发锁定，首页/History 不锁定~~ → **v1.1 修正**：MainWindow 打开即触发 `Configure`，不区分首页/二级页（见 #18） |
| 7 | History 存储 | 本地持久化，音频+文字+时间戳，FIFO 20 条，支持复制，不支持删除/搜索 |
| 8 | 错误处理 | v1 不设计，列入 Future Work |
| 9 | 生命周期/自启动 | 不开机自启；托盘由 Runtime 持有，与 MainWindow 生命周期无关 |
| 10 | 多唤醒词 | 同时只有 1 个生效唤醒词 |
| 11 | Loading 缓冲音频处理 | 无缝衔接为 Dictating 的第一批数据，不丢帧 |
| 12 | 重复触发唤醒词 | EvokeModel 仅在 Listening 运行，天然互斥，无需额外处理 |
| 13 | 敏感控件保护 | v1 不做，无条件发送到当前焦点 |
| 14 | HUD 位置 | 主屏幕顶部居中，距顶部约 80px，不跟随鼠标，仅主屏幕显示 |
| 15 | 唤醒词模型技术路线 | 自训练小型关键词检测模型（类 openWakeWord），导出 ONNX，预留声纹微调接口 |
| 16 | MainWindow 视觉风格 | 深色气泡/现代极简（VS Code / Discord 色调） |
| 17 | 托盘图标 | 跟随系统明暗主题，但不随听写状态变化；状态展示完全交给 HUD；导航为首页三卡片式 |
| 18 | MainWindow/HudWindow 关系（v1.1） | 二者由 Runtime 互斥显示，同一时刻只有一个可见（推翻此前"HudWindow常驻+MainWindow访客"的表述） |
| 19 | Configure 触发时机（v1.1 修正 #6） | 打开 MainWindow（不论首页还是任意二级页）即触发 `Configure`；若当前处于 Loading/Dictating/Unloading，会先自动流转 Unloading→Listening，再进入 Configure |
| 20 | DictationModel 触发因果（v1.1） | DictationModel 严格只能由 State Machine 在收到 EvokeModel 的唤醒词检测事件后触发加载，模块图需体现这条因果链 |
| 21 | 状态图 Configure↔Listening（v1.1） | 修正为双向转移：打开 MainWindow(→Configure) / 关闭 MainWindow(→Listening) |
| 22 | 时序图配色语义（v1.1） | 黄=唤醒词识别阶段(步骤1-2)，绿=声音采集/转换阶段(步骤3-6)，灰=不检测声音阶段(步骤7-10) |
| 23 | 状态颜色总语义（v1.2） | 黄=可被唤醒，绿=正在记录用户输入，灰=无响应状态；据此 `Loading` 定为绿灯（录音已开始，非过渡黄灯） |
| 24 | 模块图 State Machine 节点宽度（v1.2） | 大幅加宽核心节点，让周边模块可就近直连，减少连接线的长距离绕行与交叉 |
| 25 | HudWindow "可唤醒" 状态命名（v1.3） | 中文界面用语由"监听中"改为"待唤醒"，避免与"正在录音/监听"混淆 |
| 26 | MainWindow 窗口装饰（v1.3） | 不使用 OS 原生标题栏，改为自定义无边框窗口：左上角应用名"Dictating Me" + 右上角自制关闭按钮，`decorations: false` |
| 27 | MainWindow 标题栏按钮（v1.4） | 右上角关闭按钮拆分为 ▶ 播放（进入后台运行，语义同旧"关闭"）与 ⏻ 电源（退出程序）两个按钮 |
| 28 | 退出入口（v1.4） | 新增 MainWindow 电源按钮作为退出入口，无需二次确认，与系统托盘"退出"走同一路径（`Runtime::shutdown`）；此前退出只能通过系统托盘 |
| 29 | UI 视觉事实来源（v1.5） | 最新 `ui-mockup.html` 高于旧 Plan 视觉描述，是 MainWindow/HUD 结构、尺度、色彩和极简程度的唯一事实来源 |
| 30 | Configure 可见性（v1.5） | Configure 仍由 Runtime 内部执行，但不在 UI 中展示横幅、toast 或用户可见状态切换公告；后台/退出动作透明 |
| 31 | 标题栏结构（v1.5） | 高 60px；首页左侧留空，二级页显示线性 Home 图标 + 页名，History 记录数只在标题栏；永久 D 标记和应用名移除 |
| 32 | 二级页最小结构（v1.5） | InputDevice 仅设备行与选中行实时波形；EvokeWord 仅直接输入和灵敏度滑杆并自动保存；History 仅可滚动记录卡片 |
| 33 | HUD 文案（v1.5） | 运行文案固定为“待唤醒”与“听写中” |
| 34 | 文档事实来源（v1.6） | `architecture.html` 决定架构/状态/数据流；`ui-mockup.html` 决定 UI 结构/交互/视觉；Plan 只做文字汇总 |
| 35 | 架构逻辑分组（v1.6） | Text Diff + Text Injector 归入 Streaming Output；History + Config 作为持久化逻辑组 |
| 36 | 非待机打开 MainWindow（v1.6） | Loading / Dictating / Unloading 收到请求后统一完成清理再进入 Configure；重复或过期事件不改变稳定状态 |
| 37 | Runtime 退出语义（v1.6） | 托盘退出和 MainWindow 电源按钮终止进程；播放按钮和窗口关闭请求仅进入后台 |
| 38 | 输入电平显示（v1.7） | 波形采用噪声门 `0.003` + gamma `0.22`，约 `0.01→0.34`、`0.03→0.45`、`0.1→0.60` |
| 39 | 标题栏拖动（v1.7） | MainWindow 标题栏非按钮区域可拖动；Home/播放/电源按钮不触发拖动 |
| 40 | 文本注入方式（v1.7） | Streaming Output 从逐字符模拟键入改为剪贴板写入 + 带私有标记的 `Ctrl+V` |
| 41 | 默认唤醒词（v1.7） | 新配置默认“你好”；旧默认“小助手”通过配置迁移更新 |
| 42 | Windows 提权模型（v1.8） | 整个 DictatingMe 使用 `requireAdministrator`，每次启动显示 UAC，以支持管理员前台窗口的粘贴和 dismiss |
| 43 | 提权安全约束（v1.8） | 高权限 WebView 只加载受信本地/开发内容，使用受限 CSP 和最小 capability，不引入远程页面 |
| 44 | 运行提示音（v1.9） | Listening→Loading 播放 960Hz/45ms 短滴；Dictating→Unloading 播放 560Hz/55ms 短滴，均由 Runtime 非阻塞合成 |
| 45 | 提示音文件化（v1.10） | 使用内嵌 PCM WAV：wake.wav 约 1040Hz/90ms，end.wav 约 620Hz/100ms；文件可直接试听和替换 |
| 46 | Runtime 初始窗口（v1.11） | 启动初态为 Configure，MainWindow 首先显示；用户点击播放后进入后台 Listening |
| 47 | HUD 透明背景（v1.11） | Window/WebView/HTML 根背景都显式透明，桌面上不显示 HUD 外围矩形底色 |
| 48 | 误唤醒验收素材（v1.11） | 后续人工播放 MS-SNSD 的人潮、交通、轨道交通、复印机和吸尘器噪声检查误触发；本轮只筛选素材 |
| 49 | 统一资源下载脚本（v1.12） | `download-assets.ps1` 验证/修复模型与 MS-SNSD WAV；`download-assets.cmd` 负责绕过 ExecutionPolicy 并透传参数 |
| 50 | 误唤醒噪声集规模（v1.13） | `assets/noise/` 固定准备 30 个 16kHz WAV：22 个训练噪声 + 8 个独立测试噪声 |
| 51 | 顶层资源目录（v1.14） | 模型与验收素材统一迁移到 `assets/`；下载入口为根目录 `download-assets.cmd` |
| 52 | 内置唤醒模型目录（v1.15） | 初始 EvokeModel 位于 `assets/preset/` 并随软件包分发；原 `assets/evoke/` 已移除 |
| 53 | 唤醒设置模式（v1.16） | 通用文字 / 语音匹配 / 声纹验证 / 分类训练共用同一 Processor trait 和设置会话 |
| 54 | 录入规格（v1.16） | 文字 0 条；语音匹配 3 条；声纹验证 3 条；分类训练 6 条；每条由 Runtime 固定采集 5 秒 |
| 55 | 资源下载与 SHA（v1.16） | 公共 DownloadButton 使用 link list + folder path；Settings Plane 的 AssetManager 依据 `assets/sha.json` 验证并原子安装 |
| 56 | AppData 受管存储（v1.16） | `%LOCALAPPDATA%\DictatingMe` 保存 DB/profile/草稿/History；模型、训练资源及 operation 作用域 staging/trash 统一位于 `assets\` 并由 ManagedStorage 管理 |
| 57 | Profile 原子替换（v1.16） | 新 profile 处理与验证全部成功后才替换 active；失败、取消或崩溃保留旧 profile |
| 58 | 语音模型门控（v1.16，后续修订） | 唯一 DictationModel 就绪后自动选择；多个模型时手动选择。没有 active 模型或可用 EvokeProfile 时禁止进入 Listening |
| 59 | 接口简化（v1.16） | 移除独立 `set_evoke_word` 方向，改用统一 begin/capture/finish/cancel；下载和处理共用 operation/event |
| 60 | Settings Actor（v1.17） | 配置、下载、模型选择、Evoke Setup 全部进入独立 SettingsHandle；RuntimeHandle 不增加设置接口 |
| 61 | Evoke/Runtime 零直连（v1.17） | Evoke Setup 成功只提交 profile 并递增 generation，不向 Runtime 发送 DTO、路径、进度或回调 |
| 62 | RuntimeDataPort（v1.17） | 双方固定为 `subscribe_generation` / `acquire_active_bundle` / `append_history` 三个方法 |
| 63 | RuntimeBundle（v1.17） | 进入 Listening 前一次获取完整运行依赖与 leases；Listening/Loading/Dictating 不查询 Storage |
| 64 | SettingsSnapshot（v1.17） | UI 设置读取收敛到单一 snapshot；`settings-changed` 只携带 generation 并触发 refetch |

---

## 附：配套文件

- `architecture.html` —— **架构事实来源**：模块设计图 / 状态机图 / 时序图
- `ui-mockup.html` —— **UI 事实来源**：MainWindow（首页四入口、唤醒设置、语音模型）与 HudWindow 的结构、交互和视觉
- `interface-summary.html` —— 当前 Runtime / UI 类型、字段、接口和跨层事件索引；不替代上述产品设计事实来源
- `architecture-evoke-setup.html` —— **待评审 Evoke 流程图**：模块、四模式录入、Profile 激活、Runtime 最小交接与时序
- `interface-evoke-setup.html` —— **待评审 Evoke 接口图**：SettingsHandle 公共面、Processor/Verifier、Commands 与零 Runtime 直连
- `architecture-storage.html` —— **待评审 Storage 流程图**：模块、Asset 安装、引用/清理、Runtime 边界与恢复
- `interface-storage.html` —— **待评审 Storage 接口图**：SettingsSnapshot、Asset/Content、Commands 与三方法 RuntimeDataPort
