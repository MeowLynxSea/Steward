<p align="center">
  <img src="ironcowork.png?v=2" alt="IronCowork" width="200"/>
</p>

<h1 align="center">IronCowork</h1>

<p align="center">
  <strong>面向本地知识工作的桌面端自主 AI Agent</strong>
</p>

<p align="center">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache%202.0-blue.svg" alt="License: MIT OR Apache-2.0" /></a>
</p>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ru.md">Русский</a> |
  <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  <a href="#产品定位">产品定位</a> •
  <a href="#核心原则">核心原则</a> •
  <a href="#当前方向">当前方向</a> •
  <a href="#用户文档">用户文档</a> •
  <a href="#开发者启动">开发者启动</a> •
  <a href="#配置">配置</a> •
  <a href="#安全">安全</a> •
  <a href="#架构">架构</a>
</p>

---

## 产品定位

IronCowork 不是给现有 Coding CLI 套一个 GUI，也不再以“预定义工作流驱动”作为产品中心。

修正后的目标更接近 Claude Code / OpenClw 一类的自主 agent，只是运行场景是桌面端知识工作：

- 用户在一个持续存在的桌面会话里给出目标
- agent 自主浏览本地文件、工作区索引内容和 MCP 外部工具
- agent 规划并执行多步操作
- Ask/Yolo 决定高风险副作用是否需要人工批准
- 同一套后端既能在 Tauri 桌面壳中运行，也能通过浏览器访问本地 `127.0.0.1` 服务

之后可以有保存下来的 routine，但它们不再是一等公民。产品中心应该是“持续会话中的自主 agent”。

## 核心原则

- **桌面优先**：用 Tauri 提供通知、托盘、拖放等原生能力，但业务逻辑仍走 HTTP/SSE。
- **本地优先**：以 libSQL 为唯一默认存储，不依赖 PostgreSQL，不强绑云账号。
- **自主但可审查**：agent 可以连续执行，但 Ask/Yolo 和事件日志必须让高风险操作可回放、可批准、可拒绝。
- **工作区优先**：本地文件、索引文档、报告输出和 MCP 工具都是 agent 的核心上下文。
- **明确分叉**：保留原有代码库中有价值的 Rust 运行时和安全能力，但不再沿用其旧的渠道化产品思路。

## 当前方向

仓库当前已经围绕 IronCowork 的产品方向完成收口，正在补齐打包与终端用户文档。

保留：

- Rust agent loop 与调度能力
- WASM 沙箱与 prompt safety
- MCP / 工具注册表
- 工作区索引与混合检索
- 多 LLM 提供商适配

逐步删除或降级：

- 渠道式交互入口
- 面向 NEAR 账号的 onboarding
- PostgreSQL 假设
- 将 IronCowork 描述为“预定义工作流产品”的文档

新的核心对象：

- 持久化 agent 会话
- 会话中派生的执行 run / task 记录
- Ask/Yolo 批准检查点
- 可选的后台 routine
- 基于 Svelte + Axum 的统一桌面/浏览器 UI

## 功能

### 运行时

- **自主 agent 会话**：围绕真实目标执行多步操作
- **Ask/Yolo 模式**：对文件修改、删除、联网等风险动作进行拦截或放行
- **后台 routine**：在核心 agent 形态稳定后支持周期性运行
- **libSQL 本地存储**：保存设置、会话、执行记录、审批状态和工作区状态
- **工作区检索**：全文 + 向量搜索

### 安全

- **WASM 沙箱**
- **凭据边界注入与泄露检测**
- **提示注入防御**
- **网络端点白名单**
- **可审计的会话/执行/审批事件流**

### 扩展

- **MCP 支持**
- **插件/工具扩展**
- **多 LLM 后端**

## 用户文档

终端用户文档在这里：

- [docs/user-guide.md](docs/user-guide.md)：安装、浏览器模式与桌面模式、提供商配置、本地存储路径、session、审批和 workspace 使用方式
- [docs/release-readiness.md](docs/release-readiness.md)：支持的打包目标、构建命令和 release 校验清单

## 开发者启动

fresh clone 的开发者启动说明见 [docs/developer-bootstrap.md](docs/developer-bootstrap.md)。

最短路径：

```bash
./scripts/dev-setup.sh
```

这个脚本会准备 Rust/WASM 前置依赖、安装 UI 依赖、构建静态前端产物，并安装 git hooks。

日常开发命令：

- 浏览器模式：`cargo run -- api serve --port 8765`，然后访问 `http://127.0.0.1:8765`
- 桌面模式：直接运行 `cargo desktop`

## 配置

目标形态使用本地配置文件和环境变量启动：

```env
DATABASE_BACKEND=libsql
LIBSQL_PATH=~/.ironcowork/ironcowork.db
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

不应再要求 NEAR 登录或 PostgreSQL 初始化。

LLM 提供商说明见 [docs/LLM_PROVIDERS.md](docs/LLM_PROVIDERS.md)。

## 安全

IronCowork 会把原有的纵深防御继续应用到桌面自主执行场景：

- 高风险副作用必须走工具层和安全层
- Ask 模式可在真正落盘或联网前挂起等待批准
- Yolo 模式仍然受相同策略和沙箱约束
- 本地优先不等于允许任意 shell 执行
- 密钥仍然不能暴露给工具运行环境

## 架构

```
+------------------------+      HTTP/SSE      +------------------------+
|  Svelte UI             | <----------------> |  Axum API              |
|  - sessions            |                    |  默认监听 127.0.0.1    |
|  - runs                |                    |  settings/sessions     |
|  - approvals           |                    |  tasks/workspace       |
+-----------+------------+                    +-----------+------------+
            |                                             |
            | 可选 Tauri 桌面壳                           |
            v                                             v
+------------------------+                    +------------------------+
|  原生能力桥            |                    |  Rust runtime          |
|  通知                  |                    |  agent loop            |
|  托盘                  |                    |  tools + MCP           |
|  文件拖放              |                    |  safety + storage      |
+------------------------+                    +------------------------+
                                                         |
                                                         v
                                              +------------------------+
                                              |  libSQL                |
                                              |  本地嵌入式数据库      |
                                              +------------------------+
```

## 状态

当前文档正在统一到以下方向：

- 以桌面端自主 agent 为中心
- 以会话和执行 run 为中心
- routine 是次级能力
- 产品中心不再允许回到预定义工作流系统

## License

可在以下许可证下使用：

- Apache License, Version 2.0
- MIT license
