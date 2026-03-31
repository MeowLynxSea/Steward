# Backend Development Guidelines

> Best practices for backend development in this project.

---

## Overview

This directory contains guidelines for backend development. Fill in each file with your project's specific conventions.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module organization and file layout | To fill |
| [Database Guidelines](./database-guidelines.md) | ORM patterns, queries, migrations | To fill |
| [Desktop-First Architecture](./desktop-first-architecture.md) | Target runtime boundaries for the IronCowork fork | Active |
| [Error Handling](./error-handling.md) | Error types, handling strategies | To fill |
| [Quality Guidelines](./quality-guidelines.md) | Code standards, forbidden patterns | To fill |
| [Logging Guidelines](./logging-guidelines.md) | Structured logging, log levels | To fill |
| [Task Runtime Contracts](./task-runtime-contracts.md) | Task/template APIs, Ask/Yolo rules, SSE contracts | Active |

---

## How to Fill These Guidelines

For each guideline file:

1. Document your project's **actual conventions** (not ideals)
2. Include **code examples** from your codebase
3. List **forbidden patterns** and why
4. Add **common mistakes** your team has made

The goal is to help AI assistants and new team members understand how YOUR project works.

---

## Pre-Development Checklist For IronCowork Migration Work

Read these files before starting fork-direction implementation work:

1. [Desktop-First Architecture](./desktop-first-architecture.md)
2. [Task Runtime Contracts](./task-runtime-contracts.md)
3. [Database Guidelines](./database-guidelines.md) when touching libSQL storage and migrations
4. [Error Handling](./error-handling.md) and [Logging Guidelines](./logging-guidelines.md) when wiring new runtime paths

---

**Language**: All documentation should be written in **English**.
