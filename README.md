# GitHub Notifications notifier for Linux

This tool will send a desktop notification (using D-Bus) on Linux every time you receive a new notification in GitHub.

## Usage

This tool expects an environment variable called `GITHUB_TOKEN` with a valid Personal Access Token for GitHub which is allowed to read notifications.

Run the binary, it'll start notifying you.
Currently, it checks every 30s, this is hardcoded at the moment.

## Installation

There is an example systemd unit file in this repository: `gh-notifier.service`.
Change the path to the binary and add your GitHub token.
Copy it to `~/.config/systemd/user` and enable it:

```bash
systemctl --user daemon-reload
systemctl --user enable --now gh-notifier.service 
```
