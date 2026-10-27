# csr — fast Claude Code status line

Single-binary Rust port of the `aurora` / `graphite` look of
[claude-statusbar](https://pypi.org/project/claude-statusbar/) 3.52.2. No daemon, no network,
no transcript scanning: one process per render that reads Claude Code's stdin JSON and a few
small git files, prints two lines, exits.

```
5h ■■■■■■■■■■■■  1% ⏰4h40m →16% │ 7d ■■■■■■■■■■■■ 16% ⏰2d19h →27% │ Fable 5.1 ■■■■■■ 8% │ ❄ 49m
⎇ development ●  ·  orbit  ·  +965 −253  ·  worktree (🌳1)
```

## Build

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release          # → target/release/csr
cargo test --release
```

## Use as the Claude Code status line

In `~/.claude/settings.json` (replace the existing `statusLine` block; stop the Python daemon
first with `cs daemon stop` if it is running):

```json
"statusLine": { "type": "command", "command": "/home/someone/dev/projects/claude-statusbar-rs/target/release/csr", "refreshInterval": 1 }
```

## What it reads

- stdin: `rate_limits`, `model`, `context_window`, `prompt_cache.expires_at`, `cost`, `workspace`, `cwd`.
- `$HOME/.claude/claude-statusbar.json` (same file as the Python tool, subset of keys:
  `bar_glyph`, `density`, `show_weekly`, `show_cost`, `show_cache_age`, `show_project_branch`,
  `show_lines`, `show_duration`, `show_ahead_behind`, `show_projection`, `worktree_glyph`,
  `color_ok/warn/hot/worktree`, `warning_threshold`, `critical_threshold`). `CSR_CONFIG` overrides the path.
- git: `.git/HEAD`, `.git/worktrees/*/gitdir`, `.git/config` (origin URL for the links).
- `<cache dir>/git/<hash>.json`: dirty / ahead / behind, refreshed at most every 30 s by a
  detached `csr git-refresh <toplevel>` child running `git --no-optional-locks status`.
- `<cache dir>/rate_latest.json`: the freshest 5h/7d reading seen by any open Claude Code
  window (Claude Code only refreshes a session's quota when that session calls the API, so an
  idle window would otherwise show stale numbers). Merge rule: later reset wins; same reset keeps
  the higher percentage.

`<cache dir>` is RAM-backed on Linux (`$XDG_RUNTIME_DIR/csr`, else `/dev/shm/csr-<uid>`, mode 0700, so the
periodic writes never reach the SSD) and `$HOME/.cache/csr` on macOS or when neither is usable. The cache is
rebuilt within one refresh after a reboot.

`CSR_NOW=<unix seconds>` freezes the clock (tests only).

## Differences from the Python tool

- `→NN%` projection = plain average pace over the window so far (no smoothing, no cross-session store).
- The shared quota file is one account only and has no "limits were raised" downgrade grace
  (a lower percentage with the same reset time is kept at the higher value until the window
  rolls over); cache countdown only from `prompt_cache.expires_at`.
- Not rendered: language coach, relay balance, BYPASS, ocs, IP/fingerprint risk, mode line,
  todos/tools/agents, version/update hint, `New looks` hint.

## Re-capturing the golden fixture

`tests/golden.rs` replays the captured golden against a synthetic repo built in a temp dir, so
it does not depend on any checkout on this machine. Re-capture is only needed if the Python
renderer changes. The capture must then be made against a checkout with the same facts the test
fabricates: branch `development`, origin `git@github.com:exampleco/orbit.git`, exactly one live
linked worktree, dirty tree. The commands below are reference, not routine:

```bash
P=/home/someone/dev/projects/claude-statusbar-rs
FAKEHOME=$(mktemp -d)
HOME=$FAKEHOME COLUMNS=182 cs < $P/tests/fixtures/stdin-orbit.json > /dev/null   # fills the git cache
sleep 6
date +%s.%N > $P/tests/fixtures/golden-orbit.now
HOME=$FAKEHOME COLUMNS=182 cs < $P/tests/fixtures/stdin-orbit.json | grep -v 'New looks' > $P/tests/fixtures/golden-orbit.raw
```
