# DictatingMe 隐私政策

生效日期：2026-08-02

## 数据处理

DictatingMe 在本机处理麦克风音频和语音识别结果。项目维护者不运营用于接收
麦克风音频、听写文本、声纹、历史记录或使用分析的服务器。

应用在 `%LOCALAPPDATA%\DictatingMe` 下保存：

- 设置和当前唤醒 profile；
- 用户选择下载的语音及声纹模型；
- 最近 20 条非空听写历史及其录音；
- 本地诊断日志。

## 网络访问

只有在用户明确请求下载模型时，应用才连接
`assets/manifest-cn.json` 中列出的 GitHub、Hugging Face 镜像、
ModelScope 或其他模型源。下载服务可能按其隐私政策接收常规网络元数据，
例如 IP 地址和 User-Agent。DictatingMe 不向这些服务上传用户录音或听写
内容。

应用不包含广告、行为分析、遥测或用户账户系统。

## 用户控制

用户可以在 History 页面查看本地历史。卸载应用不会默认删除用户模型和历史，
以避免升级时丢失数据；用户可以删除 `%LOCALAPPDATA%\DictatingMe` 来清除全部
本地数据。

## 联系方式

隐私问题请通过
[GitHub Issues](https://github.com/DexterDreeeam/DictatingMe/issues) 提交，
且不要在公开 issue 中附加私人录音或敏感信息。
