# Fix live session history recovery during streaming

## Goal
Make session switching preserve the already-produced portion of a running assistant response.

## Requirements
- `get_session` must return the currently visible history even when the active thread is still running.
- In-progress assistant text must survive a session switch and continue appending into the same message.
- Completed sessions must keep their existing history behavior.

## Acceptance Criteria
- [ ] Switching into a running session shows the assistant text that was streamed before the switch.
- [ ] New chunks after the switch append onto the same assistant message instead of creating a new turn.
- [ ] Idle/completed sessions still load the same history as before.

## Technical Notes
- Persist live assistant output incrementally, similar to live thinking/tool-call persistence.
- Reuse the loaded assistant message as the streaming anchor in the Svelte session store.
