# Backend Development Guidelines

> Best practices for backend development in this project.

---

## Overview

This directory contains the backend conventions that are already active in the repository, including migration-era rules for the Steward fork.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module organization and file layout | Active |
| [Database Guidelines](./database-guidelines.md) | ORM patterns, queries, migrations | Active |
| [Desktop-First Architecture](./desktop-first-architecture.md) | Target runtime boundaries for the Steward fork | Active |
| [Error Handling](./error-handling.md) | Error types, handling strategies | Active |
| [Quality Guidelines](./quality-guidelines.md) | Code standards, forbidden patterns | Active |
| [Logging Guidelines](./logging-guidelines.md) | Structured logging, log levels | Active |
| [Task Runtime Contracts](./task-runtime-contracts.md) | Session/run APIs, Ask/Yolo rules, SSE contracts | Active |

---

These files should describe actual backend behavior and migration constraints, not placeholder best practices.

---

## Pre-Development Checklist For Steward Migration Work

Read these files before starting fork-direction implementation work:

1. [Desktop-First Architecture](./desktop-first-architecture.md)
2. [Task Runtime Contracts](./task-runtime-contracts.md)
3. [Database Guidelines](./database-guidelines.md) when touching libSQL storage and migrations
4. [Error Handling](./error-handling.md) and [Logging Guidelines](./logging-guidelines.md) when wiring new runtime paths

---

**Language**: All documentation should be written in **English**.
