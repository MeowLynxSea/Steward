//! Workspace filesystem sandbox for confining raw filesystem tools.
//!
//! When a workspace is available, all `read_file`, `write_file`, `list_dir`,
//! `move_file`, `apply_patch`, and `shell` operations should be restricted to
//! the union of the user's registered workspace allowlist `source_root` directories.
//!
//! This module provides `WorkspaceSandbox`, a simple policy object that performs
//! canonical path containment checks against one or more sandbox roots.

use std::path::{Path, PathBuf};

use crate::tools::builtin::path_utils::canonicalize_stripped;
use crate::tools::tool::ToolError;
use crate::workspace::Workspace;

/// A filesystem boundary defined by the union of one or more canonical roots.
///
/// Paths are allowed if they reside under *at least one* root after
/// canonicalization (symlinks resolved).
#[derive(Debug, Clone)]
pub struct WorkspaceSandbox {
    roots: Vec<PathBuf>,
}

impl WorkspaceSandbox {
    /// Build a sandbox from the current allowlists of a workspace.
    pub async fn from_workspace(workspace: &Workspace) -> Result<Self, ToolError> {
        let summaries = workspace
            .list_allowlists()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to list allowlists: {e}")))?;

        let mut roots = Vec::with_capacity(summaries.len());
        for summary in summaries {
            let root = PathBuf::from(&summary.allowlist.source_root);
            let canonical = canonicalize_stripped(&root)
                .unwrap_or_else(|_| crate::tools::builtin::path_utils::normalize_lexical(&root));
            roots.push(canonical);
        }

        Ok(Self { roots })
    }

    /// Build a sandbox directly from a set of roots (for testing).
    #[cfg(test)]
    pub fn from_roots(roots: Vec<PathBuf>) -> Self {
        let roots = roots
            .into_iter()
            .map(|root| {
                canonicalize_stripped(&root)
                    .unwrap_or_else(|_| crate::tools::builtin::path_utils::normalize_lexical(&root))
            })
            .collect();
        Self { roots }
    }

    /// Check whether a path is contained within the sandbox union.
    ///
    /// The path is canonicalized before comparison. For non-existent paths,
    /// we walk up to the nearest existing ancestor, canonicalize it (resolving
    /// symlinks), and re-append the remaining tail. This prevents symlink
    /// escapes where a symlink inside the sandbox points outside.
    pub fn contains(&self, path: &Path) -> bool {
        if self.roots.is_empty() {
            // No roots means no sandbox boundary; deny everything as a safe default.
            return false;
        }

        let canonical = if path.exists() {
            canonicalize_stripped(path)
                .unwrap_or_else(|_| crate::tools::builtin::path_utils::normalize_lexical(path))
        } else {
            // Walk up to the nearest existing ancestor directory, canonicalize it
            // (resolving symlinks), then re-append the remaining tail.
            let mut ancestor = path;
            let mut tail_parts: Vec<&std::ffi::OsStr> = Vec::new();
            loop {
                if ancestor.exists() {
                    let canonical_ancestor =
                        canonicalize_stripped(ancestor).unwrap_or_else(|_| ancestor.to_path_buf());
                    let mut result = canonical_ancestor;
                    for part in tail_parts.into_iter().rev() {
                        result = result.join(part);
                    }
                    break result;
                }
                if let Some(name) = ancestor.file_name() {
                    tail_parts.push(name);
                }
                match ancestor.parent() {
                    Some(parent) if parent != ancestor => ancestor = parent,
                    _ => break crate::tools::builtin::path_utils::normalize_lexical(path),
                }
            }
        };

        self.roots.iter().any(|root| canonical.starts_with(root))
    }

    /// Return the first root, useful for choosing a default working directory.
    pub fn first_root(&self) -> Option<&Path> {
        self.roots.first().map(|p| p.as_path())
    }

    /// Return all roots.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_contains_allows_inside_single_root() {
        let dir = tempdir().unwrap();
        let sandbox = WorkspaceSandbox::from_roots(vec![dir.path().to_path_buf()]);

        let inside = dir.path().join("subdir/file.txt");
        assert!(sandbox.contains(&inside));
    }

    #[test]
    fn test_contains_allows_inside_union_root() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();
        let sandbox = WorkspaceSandbox::from_roots(vec![
            dir_a.path().to_path_buf(),
            dir_b.path().to_path_buf(),
        ]);

        assert!(sandbox.contains(&dir_a.path().join("a.txt")));
        assert!(sandbox.contains(&dir_b.path().join("b.txt")));
    }

    #[test]
    fn test_contains_denies_outside() {
        let dir = tempdir().unwrap();
        let sandbox = WorkspaceSandbox::from_roots(vec![dir.path().to_path_buf()]);

        let outside = std::env::temp_dir().join("steward-sandbox-test-outside.txt");
        assert!(!sandbox.contains(&outside));
    }

    #[test]
    fn test_contains_denies_traversal() {
        let dir = tempdir().unwrap();
        let sandbox = WorkspaceSandbox::from_roots(vec![dir.path().to_path_buf()]);

        let escaped = dir.path().join("../outside.txt");
        assert!(!sandbox.contains(&escaped));
    }

    #[test]
    fn test_contains_denies_symlink_escape() {
        let dir = tempdir().unwrap();
        let sandbox = WorkspaceSandbox::from_roots(vec![dir.path().to_path_buf()]);

        // Create a symlink inside the sandbox pointing outside
        let outside = std::env::temp_dir().join("steward-sandbox-symlink-escape");
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&outside).unwrap();
        let symlink = dir.path().join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &symlink).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&outside, &symlink).unwrap();

        let via_symlink = symlink.join("secret.txt");
        assert!(!sandbox.contains(&via_symlink));

        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn test_empty_roots_denies_everything() {
        let sandbox = WorkspaceSandbox::from_roots(vec![]);
        let dir = tempdir().unwrap();
        assert!(!sandbox.contains(&dir.path().join("any.txt")));
    }
}
