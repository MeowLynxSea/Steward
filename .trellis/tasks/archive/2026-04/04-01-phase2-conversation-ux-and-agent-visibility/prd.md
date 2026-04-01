# Phase 2 Conversation UX And Agent Visibility

## Goal

Make the session conversation surface the primary place where users can follow autonomous agent progress without losing access to deeper run detail.

## Scope

- Session conversation UX
- Agent progress visibility in the main chat surface
- Current plan, current action, and approval visibility
- Drill-down linkage between session and run detail

## Requirements

- The chat surface should remain the product center while exposing agent execution state clearly.
- Users should be able to see the current plan, current action, pending approval state, and final outputs without scraping raw logs.
- Run detail should feel like a drill-down from the active conversation rather than a separate product mode.
- The UI should avoid workflow-specialized framing and keep the general agent mental model intact.

## Acceptance Criteria

- [ ] Users can follow agent progress from the active session view without switching mental models.
- [ ] The session view exposes current plan, current action, and pending approval visibility in a product-safe way.
- [ ] Run detail remains available as a drill-down path from the conversation experience.
- [ ] Final outputs and approval state are legible after refresh and restart.

## Out Of Scope

- Token-by-token raw reasoning disclosure
- Large visual redesign detached from runtime clarity
