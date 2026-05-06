use anyhow::{anyhow, Context, Result};
use cap_directories::{ambient_authority, ProjectDirs};
use cap_primitives::fs::OpenOptions;
use chrono::{DateTime, Utc};
use notify_rust::Notification as SystemNotification;
use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION, IF_MODIFIED_SINCE, LAST_MODIFIED, USER_AGENT};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::io::Write;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, trace, warn};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

const GITHUB_API: &str = "https://api.github.com/notifications";
const GITHUB_API_VERSION: &str = "2022-11-28";
const USER_AGENT_VALUE: &str = concat!("gh-notifier/", env!("CARGO_PKG_VERSION"));
const LAST_UPDATED_STATE_FILE: &str = "last_updated";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(60);
const MIN_POLL_INTERVAL: Duration = Duration::from_secs(15);
const ERROR_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct Notifier(Arc<NotifierInner>);

struct NotifierInner {
    token: String,
    client: Client,
    repo_to_daemon_id: Mutex<HashMap<String, u32>>,
}

// https://docs.github.com/en/rest/activity/notifications?apiVersion=2022-11-28#about-notification-reasons
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
enum Reason {
    Approval,
    Assign,
    Author,
    Comment,
    CiActivity,
    Invitation,
    Manual,
    Mention,
    ReviewRequested,
    SecurityAdvisoryCredit,
    SecurityAlert,
    StateChange,
    Subscribed,
    TeamMention,
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug)]
struct DetailItem {
    html_url: String,
}

#[derive(Deserialize, Debug)]
struct Notification {
    id: String,
    reason: Reason,
    repository: Repository,
    subject: Subject,
    updated_at: DateTime<Utc>,
}

#[derive(Deserialize, Debug)]
struct Repository {
    full_name: String,
}

#[derive(Deserialize, Debug, Clone)]
struct Subject {
    title: String,
    url: Option<String>,
    latest_comment_url: Option<String>,
    #[serde(rename = "type")]
    subject_type: String,
}

struct FetchResult {
    notifications: Vec<Notification>,
    last_modified: Option<String>,
    poll_interval: Duration,
}

impl Notifier {
    pub fn new(token: String) -> Self {
        Notifier(Arc::new(NotifierInner {
            token,
            client: Client::new(),
            repo_to_daemon_id: Mutex::new(HashMap::new()),
        }))
    }

    pub async fn run(self) {
        let mut last_updated = read_last_updated();
        let mut last_modified: Option<String> = None;
        info!("notifier started; last seen update: {}", last_updated);

        loop {
            let interval = match self.fetch_github_notifications(last_modified.as_deref()).await {
                Ok(result) => {
                    trace!("fetched {} notifications", result.notifications.len());
                    last_modified = result.last_modified;

                    let mut max_seen = last_updated;
                    for notification in result.notifications {
                        if notification.updated_at > last_updated {
                            if notification.updated_at > max_seen {
                                max_seen = notification.updated_at;
                            }
                            let me = self.clone();
                            tokio::spawn(async move {
                                me.handle_notification(notification).await;
                            });
                        } else {
                            trace!("ignoring old notification {}", notification.id);
                        }
                    }

                    if max_seen > last_updated {
                        last_updated = max_seen;
                        if let Err(e) = write_last_updated(last_updated) {
                            warn!("failed to persist last_updated: {:#}", e);
                        }
                    }

                    result.poll_interval
                }
                Err(e) => {
                    error!("error fetching notifications: {:#}", e);
                    ERROR_BACKOFF
                }
            };

            sleep(interval).await;
        }
    }

    async fn fetch_github_notifications(
        &self,
        if_modified_since: Option<&str>,
    ) -> Result<FetchResult> {
        trace!("fetching notifications");
        let mut req = self
            .0
            .client
            .get(GITHUB_API)
            .header(AUTHORIZATION, format!("Bearer {}", self.0.token))
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header(USER_AGENT, USER_AGENT_VALUE);
        if let Some(value) = if_modified_since {
            req = req.header(IF_MODIFIED_SINCE, value);
        }

        let response = req.send().await.context("sending notifications request")?;

        let poll_interval = response
            .headers()
            .get("X-Poll-Interval")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
            .map(|d| d.max(MIN_POLL_INTERVAL))
            .unwrap_or(DEFAULT_POLL_INTERVAL);

        let last_modified = header_string(response.headers().get(LAST_MODIFIED));

        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(FetchResult {
                notifications: Vec::new(),
                last_modified,
                poll_interval,
            });
        }

        let response = response
            .error_for_status()
            .context("github notifications endpoint returned error status")?;
        let notifications = response
            .json::<Vec<Notification>>()
            .await
            .context("decoding notifications response")?;

        Ok(FetchResult {
            notifications,
            last_modified,
            poll_interval,
        })
    }

    async fn handle_notification(self, notification: Notification) {
        debug!(
            "notifying about '{}' ('{}')",
            notification.id, notification.subject.title
        );

        let repo = notification.repository.full_name;
        let body = format!(
            "{} ({}/{:?})",
            notification.subject.title, notification.subject.subject_type, notification.reason
        );
        let subject = notification.subject;
        let notif_id = notification.id;

        let prev_id = self
            .0
            .repo_to_daemon_id
            .lock()
            .ok()
            .and_then(|m| m.get(&repo).copied());

        let inner = self.0.clone();
        let repo_for_log = repo.clone();
        let notif_id_for_log = notif_id.clone();

        let clicked = tokio::task::spawn_blocking(move || {
            let mut builder = SystemNotification::new();
            builder
                .summary(&repo)
                .appname("GitHub")
                .body(&body)
                .icon("github")
                .action("default", "Open")
                .action("open", "Open in browser");
            if let Some(id) = prev_id {
                debug!("replacing previous notification {} for repo {}", id, repo);
                builder.id(id);
            }

            let handle = match builder.show() {
                Ok(h) => h,
                Err(e) => {
                    error!("failed to show notification {}: {}", notif_id, e);
                    return false;
                }
            };

            if let Ok(mut map) = inner.repo_to_daemon_id.lock() {
                map.insert(repo, handle.id());
            }

            let mut clicked = false;
            handle.wait_for_action(|action| {
                debug!("notification {} action received: {:?}", notif_id, action);
                if action == "default" || action == "open" {
                    clicked = true;
                }
            });
            clicked
        })
        .await
        .unwrap_or(false);

        if clicked {
            info!(
                "notification {} ({}) clicked, opening browser",
                notif_id_for_log, repo_for_log
            );
            if let Err(e) = self.open_browser(&subject).await {
                error!("failed to open browser for {}: {:#}", notif_id_for_log, e);
            }
        } else {
            debug!("notification {} dismissed without action", notif_id_for_log);
        }
    }

    async fn open_browser(&self, subject: &Subject) -> Result<()> {
        let api_url = subject
            .latest_comment_url
            .as_deref()
            .or(subject.url.as_deref())
            .ok_or_else(|| anyhow!("notification has no API url"))?;

        trace!("resolving html url for '{}': {}", subject.title, api_url);

        let detail = self
            .0
            .client
            .get(api_url)
            .header(AUTHORIZATION, format!("Bearer {}", self.0.token))
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header(USER_AGENT, USER_AGENT_VALUE)
            .send()
            .await
            .context("fetching notification subject")?
            .error_for_status()
            .context("notification subject endpoint returned error status")?
            .json::<DetailItem>()
            .await
            .context("decoding notification subject")?;

        tokio::task::spawn_blocking(move || {
            Command::new("xdg-open")
                .arg(&detail.html_url)
                .spawn()
                .with_context(|| format!("spawning xdg-open for {}", detail.html_url))
        })
        .await
        .context("xdg-open task panicked")??;

        Ok(())
    }
}

fn header_string(value: Option<&HeaderValue>) -> Option<String> {
    value.and_then(|v| v.to_str().ok()).map(str::to_owned)
}

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("com.github", "lfrancke", "gh-notifier", ambient_authority())
}

fn write_last_updated(datetime: DateTime<Utc>) -> Result<()> {
    let proj_dirs = project_dirs().context("locating project cache dir")?;
    let cache_dir = proj_dirs.cache_dir().context("opening cache dir")?;
    let mut state_file = cache_dir
        .open_with(
            LAST_UPDATED_STATE_FILE,
            OpenOptions::new().create(true).write(true).truncate(true),
        )
        .context("opening last_updated state file")?
        .into_std();
    write!(state_file, "{}", datetime.to_rfc3339()).context("writing last_updated")?;
    Ok(())
}

fn read_last_updated() -> DateTime<Utc> {
    let Some(proj_dirs) = project_dirs() else {
        return Utc::now();
    };
    let Ok(cache_dir) = proj_dirs.cache_dir() else {
        return Utc::now();
    };
    match cache_dir.read_to_string(LAST_UPDATED_STATE_FILE) {
        Ok(contents) => DateTime::parse_from_rfc3339(contents.trim())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        Err(_) => Utc::now(),
    }
}

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("gh_notifier=info"));
    let subscriber = FmtSubscriber::builder().with_env_filter(filter).finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default tracing subscriber failed");

    let token = match resolve_token() {
        Ok(t) => t,
        Err(e) => {
            error!("could not obtain a GitHub token: {:#}", e);
            std::process::exit(1);
        }
    };

    Notifier::new(token).run().await;
}

fn resolve_token() -> Result<String> {
    if let Ok(t) = env::var("GITHUB_TOKEN") {
        if !t.is_empty() {
            return Ok(t);
        }
    }

    debug!("GITHUB_TOKEN not set; falling back to `gh auth token`");
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .context("executing `gh auth token` (install gh or set GITHUB_TOKEN)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("`gh auth token` failed: {}", stderr.trim()));
    }

    let token = String::from_utf8(output.stdout)
        .context("`gh auth token` returned non-utf8 output")?
        .trim()
        .to_owned();

    if token.is_empty() {
        return Err(anyhow!("`gh auth token` returned an empty token"));
    }

    info!("using token from `gh auth token`");
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/notifications.json");

    #[test]
    fn parses_real_world_response_shape() {
        let parsed: Vec<Notification> = serde_json::from_str(FIXTURE).expect("parse");
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].repository.full_name, "octocat/Hello-World");
        assert_eq!(parsed[0].subject.subject_type, "PullRequest");
    }

    #[test]
    fn unknown_reason_falls_through_to_other() {
        let parsed: Vec<Notification> = serde_json::from_str(FIXTURE).unwrap();
        assert!(matches!(parsed[3].reason, Reason::Other));
    }

    #[test]
    fn null_subject_urls_become_none() {
        let parsed: Vec<Notification> = serde_json::from_str(FIXTURE).unwrap();
        assert!(parsed[1].subject.url.is_none());
        assert!(parsed[1].subject.latest_comment_url.is_none());
    }

    #[test]
    fn updated_at_parses_to_utc_datetime() {
        let parsed: Vec<Notification> = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(
            parsed[0].updated_at.to_rfc3339(),
            "2024-01-15T12:34:56+00:00"
        );
    }

    #[test]
    fn every_documented_reason_has_a_variant() {
        // From https://docs.github.com/en/rest/activity/notifications
        for reason in [
            "approval",
            "assign",
            "author",
            "comment",
            "ci_activity",
            "invitation",
            "manual",
            "mention",
            "review_requested",
            "security_advisory_credit",
            "security_alert",
            "state_change",
            "subscribed",
            "team_mention",
        ] {
            let json = format!("\"{}\"", reason);
            let r: Reason = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("reason {} failed: {}", reason, e));
            assert!(
                !matches!(r, Reason::Other),
                "documented reason {} should not deserialize to Other",
                reason
            );
        }
    }
}
