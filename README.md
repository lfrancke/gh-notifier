# GitHub Notifications notifier for Linux

Sends a desktop notification (via D-Bus) every time you receive a new GitHub
notification. Click "Open" on the notification to open the underlying issue,
PR, or comment in your browser.

## Configuration

| Variable       | Required | Purpose                                                      |
|----------------|----------|--------------------------------------------------------------|
| `GITHUB_TOKEN` | yes      | A GitHub PAT with permission to read notifications.          |
| `RUST_LOG`     | no       | Standard `tracing-subscriber` filter (default: `gh_notifier=info`). |

For a classic PAT you only need the `notifications` scope. For a fine-grained
token, grant read access to "Notifications" (and any private repos whose
notifications you care about).

The poll interval is taken from GitHub's `X-Poll-Interval` header (typically
60 seconds). `If-Modified-Since` is used to avoid spending rate-limit budget
on empty polls.

The last-seen timestamp is persisted under your user cache directory (e.g.
`~/.cache/gh-notifier/last_updated`) so restarts don't re-notify you about
old items.

## Installation

Build with `cargo build --release`. An example systemd user unit is included
as `gh-notifier.service`. Edit the `ExecStart` path and add your token, then:

```bash
cp gh-notifier.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now gh-notifier.service
```

## Notes on click-to-open

Clicking the action button opens the corresponding URL via `xdg-open`. The
"default" action is also wired up, but whether a body click triggers it is
notification-daemon-dependent (dunst/mako: yes, GNOME: usually requires the
action button). Make sure `xdg-utils` is installed and that `xdg-open` opens
your browser correctly.
