# IronCowork Post-Phase-1 Roadmap

## Goal

Define the executable roadmap for work after the initial API/UI/Tauri shell landing so the repository can move from structural migration into real MVP product delivery.

## What I Already Know

- Phase 0 and the first pass of Phase 1 have been implemented in code.
- The current frontend and Tauri shell are best described as usable scaffolding, not finished product workflows.
- The next milestone must focus on template-driven task execution, not adding more generic chat surface area.

## Assumptions

- The user wants a persisted roadmap in the repository, not code changes to product features in this turn.
- The roadmap should align with the already landed work rather than repeat the original migration plan unchanged.
- Trellis task docs should be detailed enough to drive future issue-by-issue execution.

## Requirements

- Record the current baseline after Phase 1.5.
- Break the remaining work into detailed phase and issue groups.
- Preserve the original product red lines: desktop-first, libSQL-only, HTTP/SSE-first, Ask/Yolo enforced at runtime.
- Separate MVP-critical work from stabilization and release work.
- Define test gates and phase exit criteria so later implementation can be judged against something concrete.

## Acceptance Criteria

- [ ] A project-level roadmap exists under `docs/plans/` for post-Phase-1 work.
- [ ] The roadmap includes Phase 1 closeout, Phase 2 MVP delivery, Phase 3 stabilization, and Phase 4 packaging/release readiness.
- [ ] The roadmap includes issue-level work items, acceptance expectations, and test gates.
- [ ] A Trellis task entry exists so future sessions can discover and reference this planning work.

## Out Of Scope

- Implementing templates, archive workflows, briefing workflows, or scheduler code in this turn.
- Rewriting backend spec contracts that are already captured unless they need roadmap cross-reference.
- Manual runtime validation itself; this turn only plans it.
