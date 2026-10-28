# csr: a Claude Code status line that costs almost nothing to run

`csr` is a single Rust binary (~640 KiB, two crates of dependencies: `serde` / `serde_json` and
`unicode-width`) that draws the `aurora` look of
[claude-statusbar](https://github.com/leeguooooo/claude-code-usage-bar) 3.52.2: 5-hour and 7-day
quota bars with reset countdowns and pace projection, model with its reasoning effort and a
context bar, prompt-cache countdown, and a git line.

```
5h ■■■■■■■■■■■■  1% ⏰4h40m →16% │ 7d ■■■■■■■■■■■■ 16% ⏰2d19h →27% │ Fable 5.1 xhigh ■■■■■■ 8% │ ❄ 49m
⎇ development ●  ·  orbit  ·  +965 −253  ·  worktree (🌳1)
```

Each render is one short process: it reads the JSON Claude Code sends on stdin plus a few small
files, prints two lines and exits. It runs no daemon, makes no network calls, never reads the
transcript, and never runs git while drawing.

## Strengths

- **Faster and smaller than the four published status lines measured.** A render takes 3.8 ms and
  2.9 ms of CPU. A Rust hello-world on the same machine takes 2.6 ms and 1.8 ms, so `csr`'s own
  work is about 1 ms. The next fastest, CCometixLine 2.0.1, takes about 2× the time and 4× the
  memory; the Python original takes 23× the time. See [Benchmarks](#benchmarks).
- **Flat 2 MiB of memory**, whatever the transcript size (it never reads the transcript) and
  whatever the hook payload size (the hook streams its input).
- **Git never blocks a render.** The `●` (uncommitted changes) and ahead/behind facts come from a
  cache that a detached `git status` child fills in the background.
- **An idle session runs no git at all.** Before trusting the cache, each render fingerprints
  the worktree from `.git/index` and a few `lstat`s, which takes about 1 µs per tracked file. The
  fingerprint covers edits, new and deleted files, commits, upstream moves and ignore-rule
  changes. While it is unchanged the cached answer is reused, with a re-check every 10 minutes
  as a backstop. A change refreshes the cache at once instead of waiting for a timer.
- **Tool calls show up one render sooner with the optional hook.** A `PostToolUse` hook
  (`csr hook`) refreshes the git cache right after Claude edits, writes, commits or pushes,
  instead of waiting for the next render to notice. It runs in the background, writes nothing
  when nothing changed, and waits for a refresh already in flight rather than racing it.
- **One refresh a minute is enough.** Claude Code already re-renders on every message, when the
  prompt cache expires and when a quota window resets. The timer only moves the minute
  countdowns along. With `refreshInterval: 60`, an idle session costs about 3 ms of CPU a
  minute.
- **Six built-in themes, every colour configurable, no extra cost.** The themes are `graphite`
  (the default), `nord`, `dracula`, `gruvbox`, `catppuccin` and `solarized-light`. Pick one with
  `csr theme <name>`, or preview them all with `csr themes`. Any of the eight colours can be
  overridden on top of the chosen theme. Choosing a palette takes nanoseconds: renders measured
  the same with and without themes (3.5–3.7 ms, 2.1 MiB).
- **Quota stays fresh across windows.** Claude Code updates a session's 5h/7d numbers only when
  that session calls the API. `csr` shares the freshest reading between all open windows through
  a small cache file, so an idle window does not show stale quota.
- **Tested on macOS and Linux.** It has 89 unit and integration tests, including a
  byte-for-byte golden test against the Python original. The cache directory is RAM-backed on
  Linux (`$XDG_RUNTIME_DIR` or `/dev/shm`), so periodic writes never reach the SSD.

## Weaknesses and limits

Read these before choosing `csr`.

- **One layout.** Themes change colours only. The segments, their order and the separators are
  always the aurora layout: no powerline, no widget picker, no TUI. Beyond the theme, the only
  settings are on/off switches, spacing, bar glyph and thresholds (see
  [Configuration](#configuration)). If you want to arrange your own segments, use
  [ccstatusline](https://github.com/sirmalloc/ccstatusline),
  [CCometixLine](https://github.com/Haleclipse/CCometixLine) or
  [cship](https://github.com/stephenleo/cship).
- **Fewer segments than the alternatives.** `csr` has no token counts or speeds, no session cost
  rate, PR/CI status, memory, Claude Code version, custom commands or subagent line. From the
  Python original it drops the language coach, relay balance, IP checks, the mode line and the
  todos/tools/agents rows. Cost and session duration exist but are off by default, and so is
  ahead/behind (`show_ahead_behind`).
- **Build from source only.** There are no release binaries, installer or package. You need a
  Rust toolchain (1.87 or newer), and you write the `settings.json` entries yourself.
- **Unix only.** It runs on macOS and Linux. Windows is not supported: the code uses Unix file
  metadata and process groups.
- **Render cost grows with repo size.** The fingerprint `lstat`s every tracked file on each
  render: about 0.3 ms at 400 files, 25 ms at 10 000 and 50 ms at 20 000. Above 20 000 index
  entries, and for submodules, split or sparse indexes, SHA-256 repos and `core.worktree`, there
  is no fingerprint. Those repos fall back to running `git status` in the background at most
  every 30 s.
- **The fingerprint has blind spots.** Two cases can leave `●` stale for up to the 10-minute
  backstop: emptying an untracked directory while keeping the directory, and editing a custom
  `core.excludesFile` in place.
- **The hook is a best effort.** It is `async`, so a `git status` slower than Claude's next reply
  (a very large repo) still shows up one render late. Commands you type yourself with `!` are
  not tool calls, so the hook does not see them.
- **Simpler numbers than the Python original.** The `→NN%` projection is a plain average pace
  with no smoothing. The prompt-cache countdown shows whole minutes (`❄ 4m`, `❄ <1m`), not
  `4m23s`. The shared quota file assumes a single account. Context and cache come only from
  Claude Code's stdin fields; there is no transcript fallback when a field is missing.
- **Small project.** It has one maintainer and no versioned releases yet. The benchmarks below
  come from one machine.

## Benchmarks

**Setup.**
- Apple M1 Pro, macOS 27.0.1, measured 2026-10-27.
- `hyperfine -N` (no shell), median of 80 runs after 5 warm-ups, so each tool's own caches are
  warm.
- Every tool got the same stdin payload: this repo (28 tracked files) and a real 5 MB
  transcript.
- Every tool ran with an empty `HOME`, so each used its default configuration.

**Caveat.** The tools do not draw the same thing. Each draws what its defaults show, so read
the table as the cost of the tool as shipped, not of identical work.

| Tool | Version | Language | Wall time | CPU (user+sys) | Peak RSS | Binary |
|---|---|---|---|---|---|---|
| **csr** | 0.1.0 | Rust | **3.8 ms** | **2.9 ms** | **2.2 MiB** | **638 KiB** |
| [CCometixLine](https://github.com/Haleclipse/CCometixLine) `ccline` | 2.0.1 | Rust | 7.2 ms | 5.8 ms | 8.8 MiB | 3.9 MiB |
| [cship](https://github.com/stephenleo/cship), quota + model + context + cost | 1.8.3 | Rust | 24.0 ms | 17.3 ms | 6.8 MiB | 3.2 MiB |
| cship, same without the `usage_limits` module | 1.8.3 | Rust | 5.7 ms | | 6.6 MiB | 3.2 MiB |
| CCometixLine `ccline` | 1.0.5 | Rust | 54.6 ms | 47.8 ms | 13.9 MiB | 6.4 MiB |
| [claude-statusbar](https://github.com/leeguooooo/claude-code-usage-bar) (pip install) | 3.52.2 | Python | 87.6 ms | 82.9 ms | 30.2 MiB | n/a |
| [ccstatusline](https://github.com/sirmalloc/ccstatusline) | 2.2.32 | Node | 242 ms | 255 ms | 92 MiB | n/a |
| *floor:* `/usr/bin/true` | | | 1.4 ms | 1.0 ms | 1.0 MiB | |
| *floor:* Rust hello-world | | | 2.6 ms | 1.8 ms | 1.7 MiB | |

Notes on each tool:

- **CCometixLine 2.0.1** is the closest to `csr`. It takes about 2× the time and 4× the memory,
  and it shows more by default, including a cache hit ratio read from the transcript.
- **CCometixLine 1.0.5** takes 14 ms outside a git repo and about 52 ms inside one, which fits
  git running on the render path. 2.0.1 moved git to the background, which matches the drop to
  7 ms.
- **cship** spent about 18 ms in its `usage_limits` module. In the test `HOME` that module
  reported that no Claude Code credential was found, so the number may differ with a real
  login. Adding a git line through Starship (`$git_branch$git_status`) did not change its time.
- **claude-statusbar** ran in the same time with its daemon started (88.5 ms; the daemon
  reported no renders). The prebuilt binary from its install script was not measured.
- **ccstatusline** is mostly Node start-up. With the author's own widget configuration it took
  468 ms instead of 242 ms.

**Repo size.** `csr`'s warm render grows with the number of tracked files:

| Tracked files | 26 | 408 | 10 000 | 20 000 |
|---|---|---|---|---|
| `csr` CPU per render | 2.6 ms | 3.6 ms | 28 ms | 55 ms |
| `git status` alone, for comparison | 9 ms | n/a | 56 ms | 102 ms |

## How it compares

The other tools' features come from their READMEs as of 2026-10-27.

| | csr | claude-statusbar | ccstatusline | CCometixLine | cship |
|---|---|---|---|---|---|
| Language | Rust | Python | TypeScript (Node/Bun) | Rust | Rust |
| 5h / 7d quota bars | yes, with pace projection | yes, with projection | yes | yes, pace bars | yes |
| Prompt-cache countdown | minutes | seconds | yes | yes, plus hit ratio | n/a |
| Git dirty / ahead-behind | yes, background and fingerprinted | yes | yes, cached with a TTL | yes, background since 2.0 | through Starship |
| Measured render (table above) | 3.8 ms | 87.6 ms | 242 ms | 7.2 ms | 5.7–24 ms |
| Look | fixed layout, 6 themes | 3 styles; 1 theme since 3.50 | widgets, powerline, TUI | 30+ segments, skins, 13 palettes, TUI | TOML, Starship modules |
| Install | build from source | pip / uv / script | `npx` / `bunx` | npm, releases | script, cargo |
| Windows | no | yes | yes | yes | yes |

**Pick `csr`** if you want the aurora look and the lowest possible per-render cost, on macOS or
Linux. **Pick something else** if you want to choose your own segments, need Windows, or prefer
a packaged install.

## Install

```bash
git clone https://github.com/duccuong2808/claude-statusbar-rs
cd claude-statusbar-rs
cargo build --release          # → target/release/csr
cargo test --release
```

In `~/.claude/settings.json`, replace the `statusLine` block. If you are coming from the Python
tool, stop its daemon first with `cs daemon stop`.

```json
"statusLine": { "type": "command", "command": "/path/to/csr", "refreshInterval": 60 }
```

Optionally, add the hook next to any existing `PostToolUse` entries:

```json
"PostToolUse": [
  { "matcher": "Edit|Write|MultiEdit|NotebookEdit|Bash",
    "hooks": [{ "type": "command", "command": "/path/to/csr hook", "async": true, "timeout": 15 }] }
]
```

Open `/hooks` once, or restart Claude Code, so it loads the new hook.

Each hook call costs the process start plus the fingerprint: about 3 ms for a few hundred tracked
files, plus `git status` when something changed. The timeout of 15 s covers the worst case: up to
6 s waiting for another refresh's lock, then `git status`, which is killed after 5 s.

## Configuration

`$HOME/.claude/claude-statusbar.json` is the same file the Python tool uses; `csr` reads a subset
of its keys. Set `CSR_CONFIG` to use another path.

| Key | Default | Effect |
|---|---|---|
| `show_weekly` | `true` | 7-day window |
| `show_projection` | `true` | `→NN%` end-of-window projection |
| `show_effort` | `true` | reasoning effort after the model name (`xhigh`); hidden when the model has none |
| `show_cache_age` | `true` | `❄` prompt-cache countdown |
| `show_project_branch` | `true` | branch and project on line 2 |
| `show_lines` | `true` | `+added −removed` for the session |
| `show_ahead_behind` | `false` | commits ahead of and behind upstream |
| `show_cost` | `false` | session cost |
| `show_duration` | `false` | session duration |
| `bar_glyph` | blocks | `"thin"` for a thin-line bar |
| `density` | normal | spacing around `│` |
| `worktree_glyph` | `🌳` | marker for linked worktrees |
| `theme` | `graphite` | built-in palette; an unknown name falls back to `graphite` |
| `color_*` | from the theme | override one colour, see below |
| `warning_threshold` / `critical_threshold` | `30` / `70` | percentages where bars change colour |

### Themes

```bash
csr themes             # every theme with a two-line preview, drawn with your other settings
csr theme              # the theme in use
csr theme catppuccin   # set it (other keys in the file are kept)
```

| Theme | For |
|---|---|
| `graphite` | default; cool greys, the claude-statusbar look (dark terminals) |
| `nord` | Nord palette (dark) |
| `dracula` | Dracula palette (dark) |
| `gruvbox` | Gruvbox dark (dark) |
| `catppuccin` | Catppuccin Mocha (dark) |
| `solarized-light` | Solarized light (light terminal backgrounds) |

Each colour is a hex string (`"#4ec85b"`, `"#4c5"`, `"4ec85b"`) that replaces one colour of the
chosen theme. An invalid value keeps the theme's own colour.

| Key | Colours |
|---|---|
| `color_ok` / `color_warn` / `color_hot` | severity: bars, percentages, `●`, `+/−` lines |
| `color_worktree` | linked-worktree marker; the model name is a blend of this and `color_hot` |
| `color_ink` | primary text, such as the cost |
| `color_mute` | labels, countdowns, the branch glyph |
| `color_edge` | separators and the empty cells of a bar |
| `color_line2` | line 2's text (blended with `color_mute`) |

```json
{ "theme": "nord", "color_hot": "#ff5555" }
```

Using the same file with the Python tool:
- `claude-statusbar` 3.50 and later ship only `graphite`. Given another theme name, they draw
  `graphite`.
- `cs config set` rewrites the file with only the keys it knows. It keeps `theme` and the four
  colour keys it shares with `csr` (`color_ok`, `color_warn`, `color_hot`, `color_worktree`).
- It drops `color_ink`, `color_mute`, `color_edge` and `color_line2`.

## How it works

- **stdin.** `rate_limits`, `model`, `effort.level`, `context_window`,
  `prompt_cache.expires_at`, `cost`, `workspace` and `cwd`.
- **git, read directly.** `.git/HEAD` for the branch, `.git/worktrees/*/gitdir` for live
  worktrees, `.git/config` for the origin URL used in the hyperlinks.
- **Git cache: `<cache dir>/git/<hash>.json`.** Holds dirty, ahead and behind, filled by a
  detached `csr git-refresh` child that runs `git --no-optional-locks status`, so a killed run
  never leaves `.git/index.lock` behind.
  - Each entry stores the fingerprint (`src/gitkey.rs`) taken just before its git run.
  - A render reuses the entry while the fingerprint matches (up to 10 minutes) and spawns a
    refresh the moment it differs.
  - Lock and in-flight markers keep at most one git run per repo.
  - A hung git is killed together with its process group after 5 s.
- **Shared quota: `<cache dir>/rate_latest.json`.** The freshest 5h/7d reading seen by any
  window. The later reset wins; with the same reset, the higher percentage wins.
- **`<cache dir>`.** On Linux, `$XDG_RUNTIME_DIR/csr`, else `/dev/shm/csr-<uid>` (mode 0700,
  in RAM). On macOS, or when neither is usable, `$HOME/.cache/csr`. Everything in it is rebuilt
  within one refresh after a reboot.

## Development

```bash
cargo fmt && cargo clippy --release --all-targets -- -D warnings && cargo test --release
```

`CSR_NOW=<unix seconds>` freezes the clock (tests only).

`tests/golden.rs` replays a capture of the Python renderer's output against a synthetic repo
built in a temp dir. The repo has branch `development`, origin
`git@github.com:exampleco/orbit.git`, one live linked worktree and a dirty tree. You only need
to re-capture when the Python renderer changes, and the capture must be taken from a checkout
with those same facts:

```bash
P=/path/to/claude-statusbar-rs
FAKEHOME=$(mktemp -d)
HOME=$FAKEHOME COLUMNS=182 cs < $P/tests/fixtures/stdin-orbit.json > /dev/null   # fills the git cache
sleep 6
date +%s.%N > $P/tests/fixtures/golden-orbit.now
HOME=$FAKEHOME COLUMNS=182 cs < $P/tests/fixtures/stdin-orbit.json | grep -v 'New looks' > $P/tests/fixtures/golden-orbit.raw
```
