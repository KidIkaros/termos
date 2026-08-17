# Performance baselines

Idle cost is the number every milestone's Gate defends. "Low idle" (see the M2
plan) is: one attached client, sidebar on, N idle shells, clock off => zero
timer-driven renders, bounded per-tick work, no session-list polls without a
visible consumer, idle CPU under ~0.5%.

## How to measure

- `cargo bench --bench idle_tick` — work, allocations, and ns per maintenance
  tick at idle. `work/tick` is the fraction of ticks that ran the full-window
  maintenance scans; at idle it must trend to zero.
- `cargo test --test idle -- --ignored` — asserts idle ticks take the skip path
  (no scan work), read from the `tick_stats` counter.
- `TERMOS_E2E=1 cargo test --test e2e -- --ignored test_idle_cost_stays_low` —
  boots the real binary, opens three idle shells, idles 10s, and asserts the app
  writes ~nothing to the wire (render count bounded). `TERMOS_STATS_FILE` makes
  the process dump its tick counters on clean exit.

## Numbers

`bench_idle_tick` — 3 idle daemon windows, one tick per op:

| Milestone | ns/op | B/op | allocs/op | work/tick | render/tick |
|-----------|-------|------|-----------|-----------|-------------|
| M2 baseline (48c9c51) | 470 | 568 | 9 | 1.00 | 0 |
| M2 idle diet          | 260 | 296 | 5 | 0.00 | 0 |

`test_idle_cost_stays_low` — boot + 3 windows + 10s idle:

| Milestone | idle wire bytes / 10s | ticks | work | render |
|-----------|-----------------------|-------|------|--------|
| M2 baseline (48c9c51) | 0 | 104 | 104 | 0 |
| M2 idle diet          | 0 | 104 | 1   | 0 |

Frame-skip already held at baseline (zero idle renders). The diet's win is
per-tick work: baseline ran the full-window scans on every one of the ~100 idle
ticks; the diet skips them behind a cheap gate, so `work` stays flat while
`ticks` climbs (104 idle ticks, 1 did scan work). The residual ~260 ns / 5
allocs per tick is the event loop re-arm and the update panic barrier,
not sidebar or window work.

`bench_sidebar_panel_lines_cached` — steady-state rail compose, nothing changed:
288 ns/op, 0 allocs (an unchanged frame reuses the cache). A forced rebuild is
82000 ns / 178 allocs, so a pane printing output no longer restyles the rail.
