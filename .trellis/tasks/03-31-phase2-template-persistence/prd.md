# Phase 2 Template Persistence

## Goal

Introduce task templates as first-class persisted objects so IronCowork becomes task-driven instead of chat-driven.

## Scope

- Template schema design
- Built-in vs user-created template distinction
- libSQL persistence and CRUD
- API validation rules
- Basic frontend template library scaffolding

## Requirements

- Persist template metadata, parameter schema, default mode, and output expectations.
- Support built-in templates that are inspectable and clonable, but not directly mutated.
- Support user templates with full CRUD.
- Return field-level validation failures for malformed template definitions.

## Acceptance Criteria

- [x] Template records persist and reload through the backend API.
- [x] Built-in and user templates are clearly distinguished.
- [x] CRUD routes are covered by API tests.
- [x] UI can list and inspect available templates.

## Out Of Scope

- Full execution UX
- Template scheduling
