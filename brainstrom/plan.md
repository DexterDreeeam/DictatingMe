# DictatingMe 设计方案（Brainstorm v1.4）

> 状态：头脑风暴产出，尚未进入开发排期。配套文件：`architecture.html`（模块设计图 / 状态机图 / 时序图）、`ui-mockup.html`（可点击界面预览）。

## 修订记录

**v1.4（本次修订）**：
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
2. 唤醒后，软件开始**流式**将用户语音转换为文字，并通过模拟键盘输入的方式实时"打字"到当前屏幕焦点所在的输入框；
3. 用户做出任意键盘/鼠标操作即视为结束本次听写（dismiss），软件自动收尾并回到监听状态。

核心设计原则：
- **轻量常驻**：唤醒词检测模型（EvokeModel）极小（目标 <20MB），可以长期占用内存运行；听写模型（DictationModel）体积较大，只在唤醒后按需加载，用完即卸载。
- **无感衔接**：从唤醒的一刻起就开始录音缓冲，模型加载完成后无缝处理，不丢字。
- **简单直接**：v1 阶段刻意避免过度设计（不做纠错回退、不做敏感控件保护、不做错误恢复状态机），优先把主链路做扎实。

---

## 2. 技术栈选型

| 层 | 选型 | 说明 |
| --- | --- | --- |
| Runtime（核心） | **Rust** | 对 ONNX Runtime 绑定（`ort` crate）友好，内存/性能可控，适合长期驻留进程 |
| UI | **Tauri**（Rust 后端 + Web 前端） | Runtime 与 UI 同进程，UI 通过 Tauri 的 command/event 与 Rust 后端通信，无需额外 IPC 层 |
| 目标平台 | **Windows 优先** | v1 只做 Windows；但音频采集、全局键鼠监听、模拟键盘输入、系统托盘等平台相关能力，在 Rust 侧以 trait 抽象，为未来 macOS/Linux 扩展预留接口 |

**架构含义**：因为 Runtime 和 UI 在同一个 Tauri 进程内，模块设计图里的"进程边界"其实只有一条——DM 进程 vs 操作系统。UI（MainWindow / HudWindow）是这个进程内的 WebView 窗口，通过 Tauri 的事件系统订阅 Runtime 状态变化。

---

## 3. 核心概念

### 3.1 两个 Model

| Model | 用途 | 体积/加载策略 | 技术路线 |
| --- | --- | --- | --- |
| **EvokeModel** | 只识别唤醒词 | 目标 <20MB，常驻内存，长期运行 | 自训练小型关键词检测模型（类 openWakeWord 思路），导出为 ONNX；预留"用用户声音微调"的接口（未来路线图） |
| **DictationModel** | 流式语音转文字 | 体积较大，按需加载/卸载，不长期占用资源 | 千问（Qwen）ASR，**本地 ONNX 模型**（非云端 API），离线可用 |

两者**互斥运行**：EvokeModel 只在 `Listening` 状态下处理麦克风音频；一旦进入 `Loading`/`Dictating`/`Unloading`，EvokeModel 不再监听，因此不存在"听写过程中又检测到一次唤醒词"的并发问题。

### 3.2 两个 Window（互斥显示）

| Window | 定位 | 显示条件 |
| --- | --- | --- |
| **MainWindow** | 软件的配置/设计界面（首页 InputDevice / EvokeWord / History 三卡片 + 二级页） | 仅在 `Configure` 状态显示 |
| **HudWindow** | 运行时的状态浮层（Overlay） | 在 `Listening` / `Loading` / `Dictating` / `Unloading` 状态显示（无边框、半透明、不可交互） |

**MainWindow 与 HudWindow 由 Runtime 统一控制、互斥显示**：同一时刻有且只有一个可见。触发关系是**"窗口显示由状态决定"而非"窗口决定状态"的单向依赖**，但入口动作（点击托盘图标打开 MainWindow）会反过来驱动状态切换：

- **打开 MainWindow** → Runtime 立即（或自动流转后）进入 `Configure`，同时隐藏 HudWindow
- **关闭 MainWindow**（返回/退出配置）→ Runtime 回到 `Listening`，同时显示 HudWindow

详见第 4 节状态机中 `Configure` 的双向转移。

### 3.3 Runtime

Runtime 是 DM 的核心，承担：
- 状态机驱动（见第 4 节）
- 系统托盘的创建与持有（图标固定不变，不随状态改变；左键点击唤出 MainWindow，右键菜单提供"退出"）
- 音频采集、模型加载/卸载、文字注入、全局键鼠监听、历史记录持久化、配置持久化

**关键生命周期事实**：Runtime、SystemTray 三者共生共灭；MainWindow / HudWindow 都是 Runtime 派生的"访客窗口"，二者互斥显示，均可被创建/隐藏，不影响 Runtime 主循环本身。软件**不做开机自启动**；只有通过托盘菜单显式"退出"才终止整个 Runtime 进程。

---

## 4. Runtime 状态机

### 4.1 状态定义

**颜色语义**：黄 = 可被唤醒（EvokeModel 监听中）；绿 = 正在记录用户输入（录音缓冲或流式转写）；灰 = 无响应状态（不监听也不记录）。

| 状态 | 说明 | EvokeModel | DictationModel | 显示窗口 |
| --- | --- | --- | --- | --- |
| **Configure** | MainWindow 已打开（首页三卡片或任意二级页），全局锁定 | 停止 | 不适用 | MainWindow（灰） |
| **Listening** | 默认待机状态，等待唤醒词 | 运行中 | 未加载 | HudWindow（黄灯） |
| **Loading** | 唤醒词已触发，DictationModel 正在加载；**同时立即开始录音并缓冲** | 停止 | 加载中 | HudWindow（**绿灯**，录音已开始，视为记录输入中） |
| **Dictating** | 模型加载完成，流式转写进行中；缓冲区音频无缝作为第一批输入，之后接实时流 | 停止 | 运行中 | HudWindow（绿灯） |
| **Unloading** | 用户 dismiss（任意键盘/鼠标操作，或"打开 MainWindow"请求）触发的收尾状态：停止转写、丢弃未转换内容、卸载模型 | 停止 | 卸载中 | HudWindow（灰，不再记录） |

**MainWindow 与 HudWindow 严格互斥**：`Configure` 显示 MainWindow、隐藏 HudWindow；其余四个状态显示 HudWindow、隐藏 MainWindow。

### 4.2 状态转移

```
[*] --> Listening

Listening --> Configure   : 打开 MainWindow
Configure --> Listening   : 关闭 MainWindow

Listening --> Loading     : EvokeModel 检测到唤醒词
Loading  --> Dictating    : DictationModel 加载完成（缓冲音频无缝喂入）
Loading  --> Unloading    : 打开 MainWindow 请求（中断加载，走统一清理出口）

Dictating --> Unloading   : 用户 dismiss（任意键盘键 / 鼠标左右中侧键）或"打开 MainWindow"请求
Unloading --> Listening   : 清理完成（卸载模型、丢弃未转换内容、写入 History）
```

**要点**：
- `Configure` 只能从 `Listening` 直接进入/退出（**双向转移**）；触发条件是"打开/关闭 MainWindow"这个动作本身，不再区分首页浏览还是二级页编辑——只要 MainWindow 处于打开状态，Runtime 就处于 `Configure`。
- 若用户在 `Loading` 或 `Dictating` 期间发出"打开 MainWindow"的请求（例如点击托盘图标），Runtime 会**自动、连续地**先完成 `Unloading` 收尾（丢弃未转换内容、卸载模型），流转回 `Listening`，然后立即进入 `Configure`，全程无需用户二次操作。`Unloading` 因此是所有非待机状态退出到 `Listening` 的统一出口，无论触发源是"用户 dismiss"还是"打开 MainWindow 请求"。
- `Dictating` **没有静音超时**机制——只要用户不做键鼠操作、也没有打开 MainWindow，会一直保持监听转写，哪怕中间沉默很久。这是刻意的设计取舍，兼容"说话中间停顿思考"的场景。
- `Loading` 阶段录制的音频是 `Dictating` 处理的**第一批数据**，两者无缝衔接，不丢帧。
- 详细图示见 `architecture.html`（状态机图 + 时序图 tab）。

---

## 5. 模块设计

Runtime（Rust 进程）内部划分为以下模块，详见 `architecture.html` 的"模块设计图"：

| 模块 | 职责 |
| --- | --- |
| **State Machine** | 驱动 4.1 节的 5 个状态及其转移，是整个 Runtime 的调度中枢 |
| **Audio Capture** | 麦克风设备管理与采集，支持设备切换（对应 InputDevice 设置） |
| **Audio Ring Buffer** | Loading 阶段的录音缓冲区，保证进入 Dictating 时无缝喂入 |
| **EvokeModel Engine** | 加载/运行 ONNX 唤醒词模型，仅在 Listening 态激活；**检测到唤醒词是触发 DictationModel Engine 加载的唯一入口**，两者之间是严格的先后因果关系 |
| **DictationModel Engine** | 加载/运行本地 ONNX 千问 ASR 模型；**仅由 State Machine 在收到 EvokeModel 的唤醒词检测事件后触发加载**，运行于 Loading/Dictating 态，其余时间不存在 |
| **Text Diff Engine** | 对比"上次完整识别文本"与"当前完整识别文本"，只取新增后缀（不做修正回退） |
| **Text Injector** | 通过模拟键盘输入（SendInput 等价能力）把新增文字"打字"到当前操作系统焦点控件，不使用剪贴板，不做敏感控件保护 |
| **Global Input Monitor** | 全局监听键盘/鼠标事件，用于识别 dismiss 信号 |
| **History Store** | 持久化存储最近 20 条听写记录（音频 + 文本 + 时间戳），FIFO 队列，超过 20 条自动淘汰最旧一条 |
| **Config Store** | 持久化保存 InputDevice / EvokeWord（唤醒词+敏感度）等设置 |
| **Tray Manager** | 创建/持有系统托盘图标（固定不变）；左键点击 = 向 State Machine 发出"打开 MainWindow 请求"，右键菜单提供"退出" |
| **Window Manager** | 基于 Tauri 多窗口能力，根据 State Machine 广播的状态**互斥显示** MainWindow 或 HudWindow（同一时刻只有一个可见） |

### 5.1 数据流（一次完整唤醒—听写—收尾周期）

1. `Audio Capture` 持续采集麦克风音频 → 送入 `EvokeModel Engine`（仅 Listening 态）
2. `EvokeModel Engine` 检测到唤醒词 → 通知 `State Machine` → **State Machine 才触发 `DictationModel Engine` 开始异步加载**（这是 DictationModel 被激活的唯一入口）；State Machine 同时切到 `Loading`，把麦克风音频写入 `Audio Ring Buffer`
3. 模型加载完成 → `State Machine` 切到 `Dictating`：`Audio Ring Buffer` 中的缓冲内容优先喂给 `DictationModel Engine`，随后接实时音频流
4. `DictationModel Engine` 持续产出增量识别文本 → `Text Diff Engine` 计算新增后缀 → `Text Injector` 模拟打字到当前焦点控件
5. `Global Input Monitor` 检测到任意键鼠事件，**或 `Tray Manager` 收到"打开 MainWindow"请求** → `State Machine` 切到 `Unloading`：停止喂音频、丢弃未转换内容、卸载 `DictationModel Engine`
6. 收尾完成 → 本次听写的最终文本 + 录音 + 时间戳写入 `History Store` → `State Machine` 回到 `Listening`（若收尾原因是"打开 MainWindow 请求"，则紧接着自动进入 `Configure`，`Window Manager` 随即切换显示 MainWindow、隐藏 HudWindow）

（完整时序图见 `architecture.html`）

---

## 6. MainWindow UX 设计

**整体风格**：深色气泡 / 现代极简风格（参考 VS Code / Discord 色调）。

**窗口装饰**：不使用 OS 原生标题栏（`decorations: false`），改为自定义无边框窗口：左上角显示应用名 "Dictating Me"，右上角两个自制图标按钮（悬停高亮），不提供最小化按钮：
- **▶ 播放**：进入后台运行——隐藏 MainWindow，Runtime 回到 `Listening`（待唤醒），HudWindow 随之显示；语义与"关闭窗口"完全相同，只是不再用"关闭/✕"的说法，强调"程序仍在运行、只是转入后台"。
- **⏻ 电源**：退出程序——**无需二次确认**，直接终止整个 Runtime 进程；与系统托盘右键菜单"退出"是同一操作的另一入口（此前退出只能通过系统托盘）。

**导航结构**：首页三张卡片（InputDevice / EvokeWord / History），单行布局：图标 + 标题在左，当前值右对齐显示在同一行，不用 chevron 箭头；点击进入对应二级页面，二级页面顶部用一个"小房子"图标按钮回到首页（不用左箭头，强调"回首页"而非"返回上一步"，因为二级页没有更深层级）。

**打开即锁定**：MainWindow **只要处于打开状态**（无论停留在首页还是任意二级页），Runtime 即处于 `Configure`，EvokeModel 停止、HudWindow 隐藏。这意味着用户查看 History 或首页时也无法被唤醒词唤醒——需要先关闭 MainWindow 才会恢复监听。

### 6.1 首页

- 三张卡片纵向排列：
  1. 🎤 **InputDevice** —— 副标题显示当前选中的输入设备名称
  2. 🔔 **EvokeWord** —— 副标题显示当前生效的唤醒词
  3. 📜 **History** —— 副标题显示当前记录数（如 "18/20"）
- 打开首页本身即代表 Runtime 处于 `Configure`（见上）。

### 6.2 InputDevice 二级页

- 列出系统所有可用麦克风设备，单选（radio 风格）
- 选中后可实时显示音量/波形反馈，便于确认设备生效

### 6.3 EvokeWord 二级页

- 显示当前生效唤醒词（**同一时间只能有 1 个生效**，本页是"选择/修改这一个"，不支持多唤醒词并存）
- 提供唤醒词切换入口（预设列表或自定义录制，具体交互留待下一轮细化）
- 敏感度设置（滑杆或高/中/低档位，对应 EvokeModel 的置信度阈值）
- "用你的声音微调唤醒词模型" —— 标记为 Future Work，本页先展示入口但置灰/角标"即将推出"

### 6.4 History 二级页

- 展示最近最多 20 条记录（FIFO，第 21 条进来自动淘汰最旧的 1 条）
- 每条记录包含：**时间戳 + 文本 + 音频**（可回放）
- 支持**复制**文本；**不支持删除、不支持搜索**（v1 范围明确排除）
- 打开本页同样处于 `Configure`（因为 MainWindow 已打开），不会被唤醒词唤醒，这一点与首页/其他二级页一致

---

## 7. HudWindow UX 设计

- **形态**：小尺寸、半透明、无边框浮层，不可交互（点击穿透或至少不抢焦点）
- **位置**：固定在**主屏幕**顶部居中，距离屏幕顶部约 **80px**；不跟随鼠标，多屏时只在主屏幕显示
- **与 MainWindow 互斥**：只要 MainWindow 打开（`Configure` 态），HudWindow 立即隐藏；MainWindow 关闭回到 `Listening` 及之后的状态时，HudWindow 重新出现
- **状态映射**（黄=可唤醒，绿=记录输入中，灰/隐藏=无响应）：
  - `Listening` → 黄灯
  - `Loading` → 绿灯（录音缓冲已开始，等同于"正在记录输入"）
  - `Dictating` → 绿灯
  - `Configure` → HudWindow 不存在/隐藏（MainWindow 取而代之）
  - `Unloading` → 灰/瞬时熄灭（不再记录，收尾很快，通常感知不到）
- **托盘图标**：不随状态变化，固定图标；左键点击 = 打开 MainWindow（触发 `Configure`）；所有运行时状态提示职责都交给 HUD

---

## 8. 关键机制细节

### 8.1 流式文字注入（无回退修正）

- **不使用剪贴板** copy-paste（避免污染用户剪贴板、避免"全选替换"误伤原有内容）
- 采用**模拟键盘输入**（Windows 上对应 `SendInput`，Rust 生态可用 `enigo` 等跨平台库）
- `DictationModel` 每次产出的是"当前完整识别文本"（而非单纯 delta），`Text Diff Engine` 对比上一次完整文本，只把**新增的后缀部分**模拟打字发送出去
- **不做修正回退**：如果模型后续修正了之前的识别结果（流式 ASR 常见现象），DM 不会退格重打，只会继续追加新内容——这是刻意的简化取舍，避免在任意应用中做"选中删除重打"带来的光标/焦点风险
- **无条件发送到当前操作系统焦点控件**，不做密码框等敏感控件的识别与保护（v1 明确排除，见 Future Work）

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

---

## 9. 数据持久化

| 数据 | 存储内容 | 策略 |
| --- | --- | --- |
| History | 音频文件 + 转写文本 + 时间戳 | 本地持久化，FIFO 最多 20 条，第 21 条写入时自动淘汰最旧的 1 条；支持复制文本，不支持删除/搜索 |
| Config | 当前输入设备、当前唤醒词、敏感度等 | 本地持久化，重启后保留 |

---

## 10. 未来路线图（Future Work，v1 明确不做）

1. **错误处理状态**：麦克风占用/拔出、DictationModel 加载失败、网络/资源异常等场景的独立 `Error` 状态与恢复流程
2. **敏感控件保护**：识别密码框等场景并暂停/跳过文字注入
3. **多唤醒词支持**：同时启用多个唤醒词，并可分别绑定不同行为/配置（v1 只支持单一生效唤醒词）
4. **唤醒词声纹微调**：用用户自己的声音样本对 EvokeModel 做个性化微调，提升识别率、降低误触发
5. **跨平台扩展**：macOS / Linux 的音频采集、全局键鼠监听、模拟输入、托盘能力实现（Rust 侧已预留 trait 抽象）
6. **云端 ASR 可切换**：DictationModel 除本地 ONNX 外，预留切换到 DashScope 云端流式 API 的能力（更好效果 vs 离线可用之间的取舍开关）
7. **AI 文本润色**：转写结果的标点恢复、口语化去除、AI 二次润色等（当前范围只做"忠实转写 + 追加打字"）

---

## 11. 已知风险与开放问题

| # | 风险/问题 | 说明 |
| --- | --- | --- |
| 1 | 本地 ONNX 千问 ASR 的推理性能 | 本地部署对 CPU/内存有一定要求，需要实测加载耗时（影响 `Loading` 状态的用户等待感）与流式转写延迟 |
| 2 | `SendInput` 在部分应用中可能被拦截 | 例如以管理员权限运行的窗口（UAC 提权）、部分反作弊/反外挂机制的游戏，模拟输入可能失效或被拦截 |
| 3 | 全局键鼠监听的实现方式与权限 | Windows 上通常用低级钩子（`SetWindowsHookEx`）或 Raw Input；需评估杀毒软件/安全软件的误报风险 |
| 4 | `Unloading` 的 HUD 过渡视觉 | 颜色语义已明确（黄=可唤醒/绿=记录输入/灰=无响应），Loading 已定为绿灯；`Unloading` 具体是瞬时熄灭还是有渐隐动画，细节留待下一轮 UI 细化 |
| 5 | EvokeWord 唤醒词切换的具体交互 | "选择/修改唤醒词"的界面细节（预设列表 vs 自定义录制新词）留待下一轮细化 |
| 6 | 流式识别"仅追加不回退"的体验影响 | 需要在实际使用中验证：千问 ASR 流式输出的 partial 结果修正频率是否会导致明显的"错字堆积"现象 |

---

## 12. 需求确认记录（决策日志）

本方案基于持续的头脑风暴澄清问答逐步确定，供后续回溯设计依据（v1.0 首轮共 17 条，#18 起为后续修订轮次新增）：

| # | 主题 | 结论 |
| --- | --- | --- |
| 1 | 技术栈 | Runtime: Rust；UI: Tauri（同进程） |
| 2 | DictationModel 部署 | 本地 ONNX 模型（非云端 API），离线可用 |
| 3 | 流式输出机制 | 模拟键盘输入，纯追加打字，不做修正回退，不用剪贴板 |
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
| 17 | 托盘图标 | 固定不变，状态展示完全交给 HUD；导航为首页三卡片式 |
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

---

## 附：配套文件

- `architecture.html` —— 模块设计图 / 状态机图 / 时序图（可在浏览器中打开浏览）
- `ui-mockup.html` —— MainWindow（首页+三个二级页）与 HudWindow 状态的可点击界面预览
