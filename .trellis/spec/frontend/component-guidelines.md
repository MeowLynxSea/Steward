# Frontend Component Guidelines

## Modals and Dialogs
All modals in the Svelte application should uniformly follow the styling and behaviors of the main application layout (`.app-container` in `App.svelte`) rather than creating isolated global overlays.

### Styling Rules
- **Positioning**: Use `position: absolute; inset: 0; z-index: 40;` for backdrops instead of `position: fixed;` to ensure they are visually bounded by the `.app-container` window borders.
- **Backdrop Appearance**: Standardize backdrop layers using `background: rgba(0, 0, 0, 0.28); backdrop-filter: blur(10px);`.
- **Modal Box Styling**: Use softer container radiuses like `border-radius: 24px;` rather than `12px` to match the `global-mount-modal` style.
- **Inner Small Cards**: Small internal cards inside modals should have `padding: 16px; border-radius: 12px;` and use `<h3>` for their primary copy block titles, with subtitle elements below using a muted secondary color (like `text-sm text-gray-400` equivalent).
- **Transitions**: Rely on built-in `svelte/transition` (e.g., `transition:fade={{ duration: 180 }}` for backdrops, and `in:scale={{ duration: 220, start: 0.92 }} out:scale={{ duration: 150, start: 0.96 }}` for modal bodies) instead of raw CSS `@keyframes`.

### Behavior and Events
- **Event Handling**: Ensure `Escape` key and backdrop clicks correctly dismiss the modal or their nested children independently.
