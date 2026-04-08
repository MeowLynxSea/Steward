# Frontend Settings & Modal Spec

- **全局模态遮罩**: `absolute`定位, `z-index: 40`
- **背景遮罩色**: `rgba(0, 0, 0, 0.28)`, `backdrop-filter: blur(10px)`
- **主要弹窗圆角**: `border-radius: 24px`
- **内部小卡片**: `border-radius: 12px`, padding `16px`, 副标题使用较小且颜色较浅的字体(`text-sm text-gray-400`)
- **动画过渡**: 使用 svelte 的 `fade` 和 `scale` transition。
