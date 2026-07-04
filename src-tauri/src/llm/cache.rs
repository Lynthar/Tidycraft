//! Per-asset disk cache for LLM tag suggestions.
//!
//! Granularity is one cache entry per `(asset, provider, model, prompt_version)`
//! tuple — NOT per batch. A 50-asset call can have 30 hits and 20 misses,
//! and the provider only pays for the 20. The cache key embeds the
//! thumbnail content hash so re-saving the same texture under a new
//! filename invalidates the entry; conversely, renaming the file does NOT
//! force a re-call as long as the bytes match (the prompt's path-context
//! changes, but the suggestion is good enough to reuse).
//!
//! Storage layout: `dirs::cache_dir()/tidycraft/llm/<sha256-hex>.json`.
//! Mirrors the pattern used by `thumbnail.rs` and `scan_cache.rs`.

use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::PathBuf;

use super::TagSuggestion;

fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|p| p.join("tidycraft").join("llm"))
}

/// Build the cache key from inputs that affect the LLM's response.
///
/// Components hashed:
/// - `thumbnail_hash`: SHA256 of the raw thumbnail bytes (caller computes;
///   pass `None` for text-only requests so they don't collide with vision
///   ones)
/// - `filename` + `relative_path`: included in the user prompt, so changing
///   them changes the response context
/// - `provider_id` + `model`: same input on different models gives
///   different responses; cache them separately
/// - `prompt_version`: bump in `prompts.rs` when the system prompt
///   changes meaning, to invalidate every prior entry without manual rm
/// - `context_hash`: digest of the project framing + existing-tag context
///   (see [`hash_context`]). That context is part of the user prompt, so
///   folding it in means editing tags / descriptions / sample bindings /
///   `[project]` theme/goal invalidates suggestions generated under the
///   old context instead of returning a stale hit.
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

/// Stable digest of the project framing + existing-tag context the prompt is
/// built from. Folded into every per-asset cache key (see [`cache_key`]) so
/// editing a tag's description, adding/removing tags, changing a tag's sample
/// bindings, or updating `[project]` theme/goal invalidates stale suggestions
/// instead of returning advice generated under the old context.
///
/// `serde_json` gives a deterministic encoding here — struct field order is
/// fixed and `Vec` order is preserved. The empty-context case hashes to a
/// stable value, so no-context requests share a single cache namespace.
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

/// Persist a suggestion. Errors propagate so callers can decide whether
/// to skip caching this entry (e.g. low disk space) or surface to the
/// user. Most callers ignore the error after logging — a missed cache
/// write costs at most one extra API call next time.
pub fn save(key: &str, suggestion: &TagSuggestion) -> io::Result<()> {
    let dir = cache_dir().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no system cache dir available")
    })?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{key}.json"));
    let content = serde_json::to_string(suggestion)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    // Atomic (temp + rename): a torn cache entry would fail to parse and
    // re-bill the asset on the next run.
    crate::fs_atomic::write_atomic(&path, content.as_bytes())
}

/// Remove a single cache entry. Test support: lets suggest_with_cache tests
/// clean up the entries they wrote to the real cache dir (mirroring
/// `save_then_get_roundtrip`'s cleanup) without reaching the private
/// `cache_dir` from another module.
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
        let k1 = cache_key(Some("abcd"), "x.png", "a/x.png", "claude", "sonnet", 1, "ctx");
        let k2 = cache_key(Some("abcd"), "x.png", "a/x.png", "claude", "sonnet", 1, "ctx");
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64); // SHA256 hex
    }

    #[test]
    fn key_changes_when_any_component_changes() {
        let base = cache_key(Some("abcd"), "x.png", "a/x.png", "claude", "sonnet", 1, "ctx");
        // Each change must move the key.
        assert_ne!(base, cache_key(Some("ABCD"), "x.png", "a/x.png", "claude", "sonnet", 1, "ctx"));
        assert_ne!(base, cache_key(Some("abcd"), "y.png", "a/x.png", "claude", "sonnet", 1, "ctx"));
        assert_ne!(base, cache_key(Some("abcd"), "x.png", "b/x.png", "claude", "sonnet", 1, "ctx"));
        assert_ne!(base, cache_key(Some("abcd"), "x.png", "a/x.png", "openai", "sonnet", 1, "ctx"));
        assert_ne!(base, cache_key(Some("abcd"), "x.png", "a/x.png", "claude", "haiku", 1, "ctx"));
        assert_ne!(base, cache_key(Some("abcd"), "x.png", "a/x.png", "claude", "sonnet", 2, "ctx"));
        assert_ne!(base, cache_key(None,         "x.png", "a/x.png", "claude", "sonnet", 1, "ctx"));
        // Project/tag context is part of the key too.
        assert_ne!(base, cache_key(Some("abcd"), "x.png", "a/x.png", "claude", "sonnet", 1, "ctx2"));
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
        // Folding project/tag context into the cache key only pays off if the
        // digest is stable for an unchanged tag system (cache hits survive)
        // and moves when the context actually changes (stale advice drops).
        // Determinism relies on the caller handing us a stable sample order —
        // see `TagsData::get_assets_with_tag`, which sorts for this reason.
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

    #[test]
    fn save_then_get_roundtrip() {
        // Use a unique key so this test doesn't collide with anything
        // a developer happens to have in their real cache dir.
        let key = format!("tidycraft-test-{}", uuid::Uuid::new_v4().simple());
        let suggestion = fake_suggestion("test/path.png");

        save(&key, &suggestion).expect("save should succeed");
        let loaded = get(&key).expect("get should hit");

        assert_eq!(loaded.asset_path, suggestion.asset_path);
        assert_eq!(loaded.tags.len(), 1);
        assert_eq!(loaded.tags[0].label, "character");
        assert!(matches!(loaded.tags[0].category, TagCategory::Type));

        // Cleanup so the dev cache dir doesn't accrue test artifacts.
        if let Some(dir) = cache_dir() {
            let _ = fs::remove_file(dir.join(format!("{key}.json")));
        }
    }

    #[test]
    fn get_returns_none_for_missing_key() {
        let key = format!("tidycraft-nonexistent-{}", uuid::Uuid::new_v4().simple());
        assert!(get(&key).is_none());
    }
}
