# Phase 2 Periodic Briefing And Scheduler

## Goal

Ship the second complete workflow: generate recurring Markdown briefings from MCP sources and local notes.

## Scope

- Built-in briefing template
- Source configuration for MCP and local workspace content
- Prompt assembly and report generation
- Safe Markdown output writing
- Recurring schedule model and cron execution

## Requirements

- Support one-off briefing runs and scheduled recurring runs.
- Persist schedule configuration and create independent task instances per run.
- Validate cron expressions and next-run calculation.
- Record output file paths and generation metadata in task history.

## Acceptance Criteria

- [ ] A manual run can generate a Markdown report to disk.
- [ ] A recurring schedule can create independent historical runs.
- [ ] Invalid schedule configuration fails early with actionable errors.
- [ ] Report-generation and scheduling behavior are covered by tests.

## Out Of Scope

- Broad scheduler redesign outside briefing use cases
