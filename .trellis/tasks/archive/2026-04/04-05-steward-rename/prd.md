# Rename Project: Steward → Steward

## Goal

Systematically rename all project identifiers from Steward/IronCowork/ironclaw to Steward/steward.

## Scope

### Crates
| Current | New |
|---------|-----|
| `ironcowork` (bin) | `steward` |
| `ironclaw` (lib) | `steward_core` |
| `ironclaw_common` | `steward_common` |
| `ironclaw_safety` | `steward_safety` |

### Directories
- `crates/ironclaw_common` → `crates/steward_common`
- `crates/ironclaw_safety` → `crates/steward_safety`
- `~/.ironclaw/` → `~/.steward/`

### Files to Modify
- All `Cargo.toml` files (4 main + fuzz + tools-src)
- All Rust source files (~200)
- `.env.example`
- `src-tauri/Cargo.toml`
- Shell scripts: `ironclaw.bash`, `ironclaw.fish`, `ironclaw.zsh`
- GitHub Actions workflows
- All documentation and SKILL.md files
- `CLAUDE.md`, `README.*.md`

### Remove
- `Cargo.toml` `[package.metadata.wix]`
- `Cargo.toml` `[workspace.metadata.dist]` (crates.io publish config)

## Replacement Rules

| Pattern | Replace With |
|---------|-------------|
| `ironcowork` | `steward` |
| `ironclaw` | `steward` |
| `Steward` | `Steward` |
| `STEWARD` | `STEWARD` |
| `~/.ironclaw/` | `~/.steward/` |

## Acceptance Criteria

- [ ] `cargo check --all` passes
- [ ] All crate names updated
- [ ] All Rust identifier references updated
- [ ] All environment variable references updated
- [ ] Default data directory path updated
- [ ] Shell completion scripts renamed
- [ ] CI/CD workflows updated
- [ ] Documentation updated
- [ ] Publish config removed

## Execution Order

1. Update workspace Cargo.toml (members, dependencies)
2. Rename crate directories
3. Update each crate's Cargo.toml
4. Update Rust source files (lib.rs, main.rs, modules)
5. Update environment variables and paths
6. Update shell scripts
7. Update CI/CD
8. Update documentation
9. Verify with `cargo check --all`