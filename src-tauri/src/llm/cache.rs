//! Per-asset disk cache for LLM tag suggestions, keyed per
//! `(asset, provider, model, prompt_version)` rather than per batch. Storage is
//! `dirs::cache_dir()/tidycraft/llm/<sha256-hex>.json`.

use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::PathBuf;

use super::TagSuggestion;

// Test-only override so the disk-roundtrip tests run against a tempdir instead of
// the developer's real cache. Thread-local because `cargo test` runs tests on
// separate threads. (Plain comment: rustdoc cannot document a macro invocation.)
#[cfg(test)]
thread_local! {
    static TEST_CACHE_DIR: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

fn cache_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = TEST_CACHE_DIR.with(|d| d.borrow().clone()) {
        return Some(dir);
    }
    dirs::cache_dir().map(|p| p.join("tidycraft").join("llm"))
}

/// Build the cache key from every input that affects the LLM's response:
/// thumbnail hash, filename, relative path, provider id, model, prompt version
/// and the project/tag context digest, separated by NUL bytes.
pub fn cache_key(
    thumbnail_hash: Option<&str>,
    filename: &str,
    relative_path: &str,
    provider_id: &str,
    model: &str,
    prompt_version: u32,
    context_hash: &str,
) -> String {
    let mut h = Sha256::new();
    h.update(thumbnail_hash.unwrap_or("no-thumb").as_bytes());
    h.update(b"\x00");
    h.update(filename.as_bytes());
    h.update(b"\x00");
    h.update(relative_path.as_bytes());
    h.update(b"\x00");
    h.update(provider_id.as_bytes());
    h.update(b"\x00");
    h.update(model.as_bytes());
    h.update(b"\x00");
    h.update(prompt_version.to_le_bytes());
    h.update(b"\x00");
    h.update(context_hash.as_bytes());
    hex::encode(h.finalize())
}

/// Convenience: SHA256 of arbitrary bytes (e.g. thumbnail PNG content)
/// as a lowercase hex string. Callers use this to compute the
/// `thumbnail_hash` argument for `cache_key`.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Stable digest of the project framing and existing-tag context the prompt is
/// built from, folded into every per-asset cache key so edits to tags or
/// `[project]` invalidate stale suggestions. Empty context hashes stably.
pub fn hash_context(
    project_ctx: Option<&super::project_meta::ProjectMeta>,
    existing_tags: &[super::ExistingTagContext],
) -> String {
    #[derive(serde::Serialize)]
    struct Ctx<'a> {
        project: Option<&'a super::project_meta::ProjectMeta>,
        tags: &'a [super::ExistingTagContext],
    }
    let payload = Ctx {
        project: project_ctx,
        tags: existing_tags,
    };
    serde_json::to_vec(&payload)
        .map(|bytes| hash_bytes(&bytes))
        .unwrap_or_else(|_| "ctx-serialize-error".to_string())
}

/// Read a previously saved suggestion. Returns `None` on miss, malformed
/// JSON, or any IO error — callers re-fetch on miss, so silently
/// degrading to "miss" is the right behaviour.
pub fn get(key: &str) -> Option<TagSuggestion> {
    let path = cache_dir()?.join(format!("{key}.json"));
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Persist a suggestion. Errors propagate so callers can log or surface them; a
/// missed cache write costs at most one extra API call next time.
pub fn save(key: &str, suggestion: &TagSuggestion) -> io::Result<()> {
    let dir = cache_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no system cache dir available"))?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{key}.json"));
    let content = serde_json::to_string(suggestion)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    // Atomic (temp + rename): a torn cache entry would fail to parse and
    // re-bill the asset on the next run.
    crate::fs_atomic::write_atomic(&path, content.as_bytes())
}

/// Remove a single cache entry. Test support, so `suggest_with_cache` tests can
/// clean up entries they wrote without reaching the private `cache_dir`.
#[cfg(test)]
pub(crate) fn remove(key: &str) {
    if let Some(dir) = cache_dir() {
        let _ = fs::remove_file(dir.join(format!("{key}.json")));
    }
}

/// Remove every cached suggestion. Returns the total bytes freed
/// (sum of file sizes before deletion) so the UI can render
/// "Freed N MB" feedback like the thumbnail-cache button.
pub fn clear() -> io::Result<u64> {
    let dir = match cache_dir() {
        Some(d) => d,
        None => return Ok(0),
    };
    if !dir.exists() {
        return Ok(0);
    }
    let freed = size();
    fs::remove_dir_all(&dir)?;
    Ok(freed)
}

/// Total size (bytes) of the on-disk LLM cache. Returns 0 if the dir
/// doesn't exist yet or any IO step fails — never an error, since
/// "size unknown" is treated by the UI the same as "size zero".
pub fn size() -> u64 {
    let dir = match cache_dir() {
        Some(d) => d,
        None => return 0,
    };
    if !dir.exists() {
        return 0;
    }
    fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{SuggestedTag, TagCategory};

    fn fake_suggestion(path: &str) -> TagSuggestion {
        TagSuggestion {
            asset_path: path.into(),
            tags: vec![SuggestedTag {
                label: "character".into(),
                category: TagCategory::Type,
                confidence: 0.92,
                source: crate::llm::TagSource::New,
            }],
        }
    }

    #[test]
    fn key_is_deterministic() {
        let k1 = cache_key(
            Some("abcd"),
            "x.png",
            "a/x.png",
            "claude",
            "sonnet",
            1,
            "ctx",
        );
        let k2 = cache_key(
            Some("abcd"),
            "x.png",
            "a/x.png",
            "claude",
            "sonnet",
            1,
            "ctx",
        );
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64); // SHA256 hex
    }

    #[test]
    fn key_changes_when_any_component_changes() {
        let base = cache_key(
            Some("abcd"),
            "x.png",
            "a/x.png",
            "claude",
            "sonnet",
            1,
            "ctx",
        );
        // Each change must move the key.
        assert_ne!(
            base,
            cache_key(
                Some("ABCD"),
                "x.png",
                "a/x.png",
                "claude",
                "sonnet",
                1,
                "ctx"
            )
        );
        assert_ne!(
            base,
            cache_key(
                Some("abcd"),
                "y.png",
                "a/x.png",
                "claude",
                "sonnet",
                1,
                "ctx"
            )
        );
        assert_ne!(
            base,
            cache_key(
                Some("abcd"),
                "x.png",
                "b/x.png",
                "claude",
                "sonnet",
                1,
                "ctx"
            )
        );
        assert_ne!(
            base,
            cache_key(
                Some("abcd"),
                "x.png",
                "a/x.png",
                "openai",
                "sonnet",
                1,
                "ctx"
            )
        );
        assert_ne!(
            base,
            cache_key(
                Some("abcd"),
                "x.png",
                "a/x.png",
                "claude",
                "haiku",
                1,
                "ctx"
            )
        );
        assert_ne!(
            base,
            cache_key(
                Some("abcd"),
                "x.png",
                "a/x.png",
                "claude",
                "sonnet",
                2,
                "ctx"
            )
        );
        assert_ne!(
            base,
            cache_key(None, "x.png", "a/x.png", "claude", "sonnet", 1, "ctx")
        );
        // Project/tag context is part of the key too.
        assert_ne!(
            base,
            cache_key(
                Some("abcd"),
                "x.png",
                "a/x.png",
                "claude",
                "sonnet",
                1,
                "ctx2"
            )
        );
    }

    #[test]
    fn key_does_not_collide_across_field_boundaries() {
        // Without the \x00 separators between fields, "ab"+"cd" and
        // "abcd"+"" would hash to the same key. The separators prevent
        // that.
        let a = cache_key(Some("ab"), "cd", "", "p", "m", 1, "ctx");
        let b = cache_key(Some("abcd"), "", "", "p", "m", 1, "ctx");
        assert_ne!(a, b);
    }

    #[test]
    fn hash_bytes_is_64_hex_chars() {
        let h = hash_bytes(b"hello");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_context_is_deterministic_and_sensitive() {
        // Folding context into the key only pays off if the digest is stable for an
        // unchanged tag system and moves when the context changes. Determinism
        // relies on a stable sample order — see `TagsData::get_assets_with_tag`.
        let tags = vec![crate::llm::ExistingTagContext {
            name: "hero".into(),
            description: Some("Player characters".into()),
            sample_paths: vec!["a/b.png".into(), "c/d.png".into()],
        }];
        // Same context in -> same digest out.
        assert_eq!(hash_context(None, &tags), hash_context(None, &tags));
        // Empty context is stable and shares one namespace.
        assert_eq!(hash_context(None, &[]), hash_context(None, &[]));
        // Editing the tag context moves the digest.
        let edited = vec![crate::llm::ExistingTagContext {
            name: "hero".into(),
            description: Some("CHANGED".into()),
            sample_paths: vec!["a/b.png".into(), "c/d.png".into()],
        }];
        assert_ne!(hash_context(None, &tags), hash_context(None, &edited));
    }

    /// Point this thread's cache at a fresh tempdir for the test's duration.
    /// Returns the guard — dropping it removes the directory.
    fn isolated_cache() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        TEST_CACHE_DIR.with(|d| *d.borrow_mut() = Some(dir.path().to_path_buf()));
        dir
    }

    #[test]
    fn save_then_get_roundtrip() {
        let _dir = isolated_cache();
        let key = format!("tidycraft-test-{}", uuid::Uuid::new_v4().simple());
        let suggestion = fake_suggestion("test/path.png");

        save(&key, &suggestion).expect("save should succeed");
        let loaded = get(&key).expect("get should hit");

        assert_eq!(loaded.asset_path, suggestion.asset_path);
        assert_eq!(loaded.tags.len(), 1);
        assert_eq!(loaded.tags[0].label, "character");
        assert!(matches!(loaded.tags[0].category, TagCategory::Type));
    }

    #[test]
    fn get_returns_none_for_missing_key() {
        let _dir = isolated_cache();
        let key = format!("tidycraft-nonexistent-{}", uuid::Uuid::new_v4().simple());
        assert!(get(&key).is_none());
    }
}
