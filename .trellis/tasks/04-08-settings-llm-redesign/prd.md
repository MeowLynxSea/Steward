# Settings & Backend 模态框重构

## Goal
完全重构前端的设置模态框（SettingsView）和 Backend 配置模态框（LlmConfigurationPanel settings mode），参考主页面设计风格，采用全屏抽屉式布局。

## Design Direction
**左侧滑出抽屉式**
- Settings 抽屉从屏幕左侧滑出，覆盖主内容
- Backend 管理作为二级叠加抽屉，继续从左侧滑出
- 保持逻辑功能不变，仅修改外观布局

## Layout Specs

### Settings 抽屉
| 属性 | 值 |
|------|-----|
| 宽度 | 420px |
| 圆角 | 右侧 24px，左侧 0px |
| Backdrop | `rgba(0,0,0,0.2)` + `blur(12px)` |
| z-index | 40 (backdrop) / 41 (drawer) |
| 动画 | `translateX(-100%)` → `translateX(0)`, 280ms ease-out |
| 关闭动画 | `translateX(0)` → `translateX(-100%)`, 220ms ease-in |

### Backend 管理抽屉（二级）
| 属性 | 值 |
|------|-----|
| 宽度 | 420px |
| 堆叠在 Settings 抽屉上方 |
| z-index | 42 |
| 动画 | 同 Settings |
| 返回按钮 | 顶部左侧箭头按钮 |

### 响应式
- 桌面（≥640px）: 抽屉宽度 420px
- 移动端（<640px）: 抽屉宽度 100%

## Visual Style
- 使用现有 CSS 变量（`--bg-surface`, `--border-default`, `--shadow-dropdown`）
- 卡片圆角 18px
- 按钮 hover 微上移（`translateY(-1px)`）
- 关闭按钮在右上角

## Components

### SettingsView
- 从 `App.svelte` 的条件渲染改为独立抽屉
- 内部包含：
  - 顶部 Header：标题 + 关闭按钮
  - 导航 Tab：常规 / 模型（横向排列）
  - 内容区：根据 Tab 显示对应设置
  - Backend 按钮 → 打开二级抽屉

### LlmConfigurationPanel (settings mode)
- 作为独立抽屉组件
- 顶部：返回按钮 + 标题
- 主体：Backend 列表或表单

## Acceptance Criteria
- [ ] Settings 抽屉从左侧滑入/滑出动画正常
- [ ] 主题切换功能正常
- [ ] 模型选择功能正常
- [ ] Backend 管理抽屉正常叠加
- [ ] 新增/编辑/删除 Backend 功能正常
- [ ] 响应式布局正常（移动端全宽）
- [ ] 动画流畅无卡顿
- [ ] 无 lint / typecheck 错误

## Files to Modify
- `ui/src/App.svelte` - 移除 SettingsView 渲染，改为抽屉状态
- `ui/src/views/SettingsView.svelte` - 完全重构为抽屉组件
- `ui/src/components/LlmConfigurationPanel.svelte` - settings mode 重构为抽屉
PRD_EOF 2>&1
