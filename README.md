# 比格熊 Any2Text

随比格熊一起发布的创作辅助客户端。通过可替换服务接口调用比格熊识别与文本整理，不维护第二套 ASR。首版目标覆盖 1～2 小时媒体；取回的原文、校订和稿件由客户端保存，停用或更换提供方后仍可编辑导出。

当前状态：**T00 已连通本机比格熊 Bridge。短样本请求到达识别服务，宿主因 Whisper 运行包缺件拒绝加载。** 计划与设计基线为 **v0.4，2026-11-12**。原型中的素材、任务和稿件为演示数据。

- [开发计划](docs/plans/20261112-PLAN-any2text.md)：需求、服务合同、代码证据、T00～T06 阶段与审核记录。
- [UI 交互规范](docs/design/20261112-UI-any2text.md)：五个页面、连接/断开、取回、校订、候选和导出行为。
- [可点击 UI 原型](docs/design/any2text-ui.html)：浏览器直接打开或刷新，无需构建，不访问网络；编辑仅保存在页面内。
- [A01 架构与归属](docs/design/any2text-architecture.png) / [SVG 图源](docs/design/any2text-architecture.svg)。
- [A02 受理、恢复与成果取回](docs/design/any2text-lifecycle.png) / [SVG 图源](docs/design/any2text-lifecycle.svg)。
- [A03 文稿修订与服务替换](docs/design/any2text-revisions.png) / [SVG 图源](docs/design/any2text-revisions.svg)。
- [UI 交互总图](docs/design/any2text-flow.png) / [SVG 图源](docs/design/any2text-flow.svg)。
- [校对页预览](docs/design/any2text-ui-preview.png)、[深色预览](docs/design/any2text-ui-dark-preview.png)；其余五页与连接面板图片见 UI 规范 §6。
- [原型验证结果](docs/design/prototype-validation.json)：53 项定向断言通过，仅证明演示交互，不代表真实服务或产品验收。

[T00 探针](scripts/probe-bigbear.ps1)接收本地语音识别模型配置 ID。宿主有完整运行包或另一套已就绪配置后，可用同一探针恢复短样本验证。调用命令和失败回执见计划 §5。

接手时先启动本机比格熊，通过它的模型目录确认一套已就绪的本地语音识别配置，再用探针提交短语音。探针接收本机配置 ID，配置 ID 因机器而异。首次验证成功的标准是从比格熊实际取回原文和 SRT 并落盘。运行产物保存在本机 `artifacts/t00/`，历史设计归档保存在本机 `docs/design/history/`。

后续修改沿同一 PLAN、UI 规范及 SVG/HTML 原位修订，并刷新对应预览。
