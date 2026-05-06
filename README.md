# GitHub Notifications notifier for Linux

Sends a desktop notification (via D-Bus) every time you receive a new GitHub
notification. Click "Open" on the notification to open the underlying issue,
PR, or comment in your browser.

Multiple notifications from the same repository collapse into a single popup
that updates in place, so a busy PR doesn't pile up five separate alerts.

## Token

The tool needs a GitHub token with permission to read notifications. It tries
in order:

1. `$GITHUB_TOKEN`
2. `gh auth token` (if [`gh`](https://cli.github.com) is installed and
   logged in)

For a classic PAT you only need the `notifications` scope. For a fine-grained
token, grant read access to "Notifications" (and any private repos whose
notifications you care about).

## Configuration

| Variable       | Purpose                                                              |
|----------------|----------------------------------------------------------------------|
| `GITHUB_TOKEN` | Optional; if unset, `gh auth token` is used as fallback.             |
| `RUST_LOG`     | Standard `tracing-subscriber` filter (default: `gh_notifier=info`).  |

The poll interval is taken from GitHub's `X-Poll-Interval` header (typically
60 seconds), with a 15s lower bound. `If-Modified-Since` is used to avoid
spending rate-limit budget on empty polls.

The last-seen timestamp is persisted under your user cache directory (e.g.
`~/.cache/gh-notifier/last_updated`) so restarts don't re-notify you about
old items.

## Installation

Build with `cargo build --release` (or grab a release tarball — see below).
An example systemd user unit is included as `gh-notifier.service`. Edit the
`ExecStart` path, then:

```bash
cp gh-notifier.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now gh-notifier.service
```

For the token, the unit ships with a placeholder. The recommended approach is
to drop the `Environment=GITHUB_TOKEN=...` line entirely and either:

- rely on `gh auth token`, or
- use `EnvironmentFile=%h/.config/gh-notifier/env` with `chmod 600` on that
  file containing `GITHUB_TOKEN=...`.

### Nix

A flake is provided. NixOS and Home Manager users can enable
`services.gh-notifier` directly — see `flake.nix` for the available options.

## Releases

A new release is cut by pushing a `v*` tag. The
[`release` workflow](.github/workflows/release.yml) builds an x86_64 Linux
binary, packages it together with the unit file and license, and attaches the
tarball (plus a SHA-256 checksum) to a generated GitHub Release.

```bash
# bump version in Cargo.toml, then:
git tag v0.3.0
git push origin v0.3.0
```

The CI workflow runs `fmt`, `clippy`, `test`, and `build` on every push and
pull request to `main`.

## Notes on click-to-open

Clicking the "Open" action button opens the corresponding URL via `xdg-open`.
A "default" action is also wired up; whether a body click triggers it depends
on your notification daemon:

- **dunst** by default: only **middle-click** invokes actions. Set
  `mouse_left_click = do_action, close_current` in `~/.config/dunst/dunstrc`
  if you want left-click to fire the action.
- **mako / swaync / fnott**: render the action as a clickable button.
- **GNOME / KDE**: the action shows up as a button in the notification.

Make sure `xdg-utils` is installed and that `xdg-open` opens your browser
correctly.
