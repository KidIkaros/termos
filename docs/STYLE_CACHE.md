# Style Cache

TermOS uses a style cache to avoid recreating identical `ratatui` styles on every render frame. This document describes how the cache works, why it exists, and how to use it when contributing.

> **Ported from TUIOS** (https://github.com/Gaurav-Gosain/tuios) — adapted from the Go `lipgloss` style cache to the Rust `ratatui` style model.

## Table of Contents

- [Motivation](#motivation)
- [How the Cache Works](#how-the-cache-works)
- [Cache Keys](#cache-keys)
- [Invalidation](#invalidation)
- [Performance Characteristics](#performance-characteristics)
- [Using the Cache in New Code](#using-the-cache-in-new-code)
- [Testing](#testing)
- [Related Documentation](#related-documentation)

## Motivation

Every render frame, TermOS builds dozens of `ratatui::style::Style` objects for window borders, status bars, text blocks, the sidebar, notifications, and overlays. Constructing a `Style` from scratch each frame is cheap individually, but across hundreds of panes and thousands of cells the allocations add up. The style cache memoizes the final `Style` value so repeated lookups with the same parameters return a cached copy instead of rebuilding.

The upstream Go project faced the same issue with `lipgloss` styles and solved it with a `sync.Map`-backed cache. TermOS adapts that approach to Rust using `Arc<Mutex<HashMap<…>>>`.

## How the Cache Works

The cache lives in `src/style_cache.rs` and is held by the `App` struct as an `Arc<Mutex<StyleCache>>`. The cache is a simple `HashMap` keyed by a `StyleKey` struct:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StyleKey {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub add_modifier: Modifier,
    pub sub_modifier: Modifier,
}

pub struct StyleCache {
    map: HashMap<StyleKey, Style>,
}
```

### Lookup

```rust
pub fn get(&mut self, key: &StyleKey) -> Style {
    if let Some(s) = self.map.get(key) {
        *s
    } else {
        let s = Style::default()
            .fg(key.fg)
            .bg(key.bg)
            .add_modifier(key.add_modifier)
            .remove_modifier(key.sub_modifier);
        self.map.insert(key.clone(), s);
        s
    }
}
```

`Style` is `Copy`, so the returned value is a stack copy and no further allocation is needed at the call site.

### Thread Safety

The cache is wrapped in `Arc<Mutex<StyleCache>>` because the render loop and the input handler may both need styles. The lock is held only for the duration of the `get` call, which is a hash lookup or a single insert — never a long operation.

## Cache Keys

A `StyleKey` captures the four properties that fully determine a `ratatui::style::Style`:

| Field | Type | Description |
|---|---|---|
| `fg` | `Option<Color>` | Foreground color |
| `bg` | `Option<Color>` | Background color |
| `add_modifier` | `Modifier` | Modifiers to add (bold, italic, etc.) |
| `sub_modifier` | `Modifier` | Modifiers to remove |

Colors are `ratatui::style::Color` values, which include `Reset`, `Black`, `Red`, `Rgb(r, g, b)`, and indexed 256-color variants. All of these implement `Hash` and `Eq`.

## Invalidation

The cache never needs invalidation. Styles are pure functions of their key, so a cached entry is always correct regardless of theme changes or terminal resizing. When the theme changes, new keys are produced (different `fg`/`bg` values), and the cache simply grows with new entries.

The cache does grow unboundedly across theme switches. In practice this is negligible: each entry is a few bytes, and a typical session uses at most a few hundred distinct styles. If memory becomes a concern, `StyleCache::clear()` can be called on theme change.

## Performance Characteristics

| Operation | Complexity | Notes |
|---|---|---|
| Cache hit | O(1) | Hash lookup, returns `Copy` |
| Cache miss | O(1) amortized | Build `Style`, insert |
| Memory per entry | ~32 bytes | Key + Style value |
| Typical cache size | 50–200 entries | Depends on theme and active windows |

In benchmarks on the upstream project, the style cache reduced per-frame allocation by approximately 40% in a 10-window BSP layout. TermOS sees similar gains because `ratatui` styles are cheaper to build than `lipgloss` styles but still benefit from avoiding repeated construction.

## Using the Cache in New Code

When adding new UI elements that are rendered every frame, use the cache:

```rust
use crate::style_cache::StyleKey;

let key = StyleKey {
    fg: Some(Color::Yellow),
    bg: None,
    add_modifier: Modifier::BOLD,
    sub_modifier: Modifier::empty(),
};

let style = app.style_cache.lock().unwrap().get(&key);
```

### When to Use the Cache

- **Use it** for styles computed every frame: borders, status bar segments, sidebar items, notification text.
- **Skip it** for one-off styles used in a single render path that won't recur: a transient error popup, for instance. The overhead of the mutex lock is not worth it for a style used once.

### When Not to Use the Cache

- For styles that depend on runtime data not captured in `StyleKey` (e.g., a color derived from a hash of a filename). The cache key must be deterministic.
- For styles used outside the render loop (e.g., in a test helper).

## Testing

The style cache is tested in `src/style_cache.rs` under `#[cfg(test)]`:

- `get` returns the same `Style` for the same key.
- `get` builds the correct `Style` for a new key.
- `clear` empties the map.
- Concurrent access via multiple threads does not deadlock or panic.

Run the tests:

```bash
cargo test --lib style_cache
```

## Related Documentation

- [THEMES.md](THEMES.md) - How themes define colors that feed into the cache
- [ARCHITECTURE.md](ARCHITECTURE.md) - Where the style cache sits in the module map
- [perf.md](perf.md) - Performance benchmarks and profiling
