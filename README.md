> 团队构建/发布/开发测试成品归集规范（2026-11-23）：见 [RELEASE_OUTPUTS.md](RELEASE_OUTPUTS.md)。唯一交付根为 `L:/00Dev/bgxiong-00-release`；这是团队开发交付约束，不改变外部用户安装/运行布局。

# 比格熊-any2text

随比格熊一起发布的创作辅助客户端。通过可替换服务接口调用比格熊识别与文本整理，不维护第二套 ASR。首版目标覆盖 1～2 小时媒体；取回的原文、校订和稿件由客户端保存，停用或更换提供方后仍可编辑导出。

当前已实现 T00～T06。资源机的短样本、十分钟吞吐、两小时冷重启、真实桌面导出和联合安装记录见开发计划 §5.1。计划与设计基线为 v0.4，原型使用演示数据。

- [开发计划](docs/plans/20261112-PLAN-any2text.md)：需求、服务合同、代码证据、T00～T06 阶段与审核记录。
- [UI 交互规范](docs/design/20261112-UI-any2text.md)：五个页面、连接/断开、取回、校订、候选和导出行为。
- [可点击 UI 原型](docs/design/any2text-ui.html)：浏览器直接打开或刷新，无需构建，不访问网络；编辑仅保存在页面内。
- [A01 架构与归属](docs/design/any2text-architecture.png) / [SVG 图源](docs/design/any2text-architecture.svg)。
- [A02 受理、恢复与成果取回](docs/design/any2text-lifecycle.png) / [SVG 图源](docs/design/any2text-lifecycle.svg)。
- [A03 文稿修订与服务替换](docs/design/any2text-revisions.png) / [SVG 图源](docs/design/any2text-revisions.svg)。
- [UI 交互总图](docs/design/any2text-flow.png) / [SVG 图源](docs/design/any2text-flow.svg)。
- [校对页预览](docs/design/any2text-ui-preview.png)、[深色预览](docs/design/any2text-ui-dark-preview.png)；其余五页与连接面板图片见 UI 规范 §6。
- [原型验证结果](docs/design/prototype-validation.json)：53 项定向断言通过，仅证明演示交互，不代表真实服务或产品验收。

[T00 探针](scripts/probe-bigbear.ps1)接收本地语音识别模型配置 ID。调用命令、短样本结果和失败回执见计划 §5。

在其他机器复验时，先启动本机比格熊，通过实时模型目录确认一套支持中文和段级时间戳的本地配置，再用探针显式提交该机的配置 ID。短样本验收需实际取回并读回原文和 SRT。运行产物保存在本机 `artifacts/t00/`，历史设计归档保存在本机 `docs/design/history/`。

## 桌面程序

打开 Any2Text 时自动连接已启动的比格熊；服务未就绪或连接中断时每 5 秒重试，主动断开后本次窗口不自动重连，点击连接或启动比格熊可恢复自动连接。右上角服务状态以浮层展开，不挤压正文，点击外部或按 Esc 收起。可从浮层启动同目录的比格熊。联合安装的 Any2Text 使用比格熊本地 Bridge 的认证接口。开发环境分别启动时，在比格熊启用本地 Bridge 和传输层。连接后导入本机素材，选择音轨和已就绪的语音识别模型。任务结束后点击「取回并保存」，随后可离线校订和导出。整理成稿使用比格熊设置中的默认文本模型。

自动连接与浮层的修复、定向验证和生效边界见[修复记录](docs/plans/20261114-FIX-服务自动连接与浮层.md)。源码更新后，已有打包 EXE 需要重新构建并替换后重启。

外观默认跟随比格熊，也可独立切换五套皮肤。Windows 标题栏随当前皮肤取色。音乐剧转写时，已处理的无词片段在字幕中保留源时间空档。若任务列出待核对的时间段，取回后可在原音中检查这些位置。原故障素材使用 Qwen3-ASR 2.0.4 复验，模型安装与绑定由比格熊管理。升级运行时后，可在同一素材上创建使用新版本的转写任务。

Any2Text 随比格熊一起安装，从比格熊左侧「外挂」启动。组件固定为比格熊安装目录下的 `bgxiong-any2text.exe`。转写使用比格熊的服务和同一套 FFmpeg、Python 与语音识别模型。缺少资源时，在比格熊里联网安装或导入离线资源，装好后在 Any2Text 刷新模型列表。

开发启动使用 `npm ci` 后运行 `npm run desktop -- dev`。比格熊的 `scripts/build-installer-full.ps1` 默认从同级 `bgxiong-any2text` 源码构建并校验组件，也可通过 `-Any2TextArtifactDirectory` 使用已构建的组件和摘要清单。正式发行物只有比格熊安装器，安装时申请管理员授权并允许选择目录。模型和运行时资源经比格熊自己的安装入口供应。

开发版主程序左侧“外挂”也按 EXE 同目录启动。已有 `artifacts/joint` 组件时，运行 `powershell -ExecutionPolicy Bypass -File scripts/deploy-dev-component.ps1`，校验后部署到同级主仓的 `src-tauri/target/debug/`，无需安装联合包或重新编译主程序。自定义开发输出位置用 `-HostExecutable` 指定完整主程序 EXE 路径。此命令复用已有组件，不代表重新构建最新源码；组件代码修改后先重新构建。不同摘要的已部署组件会明确报错，需关闭并保全旧文件后替换。若主程序启动时尚未启用 Any2Text Bridge 能力，保存工作并重启主程序后再连接。排错与验证见[开发版外挂入口修复记录](docs/plans/20261113-FIX-开发版外挂入口缺少组件.md)。

后续修改沿同一 PLAN、UI 规范及 SVG/HTML 原位修订，并刷新对应预览。
