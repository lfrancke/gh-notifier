use cap_directories::{ambient_authority, ProjectDirs};
use cap_primitives::fs::OpenOptions;
use chrono::{DateTime, Utc};
use notify_rust::Notification as SystemNotification;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::mpsc;
use std::{env, process::Command};
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, trace};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

const GITHUB_API: &str = "https://api.github.com/notifications";

const LAST_UPDATED_STATE_FILE: &str = "last_updated";

#[derive(Clone)]
struct Notifier {
    token: String,
    client: Client,
}

// https://docs.github.com/en/rest/activity/notifications?apiVersion=2022-11-28#about-notification-reasons
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
enum Reason {
    Assign,
    Author,
    Comment,
    CiActivity,
    Invitation,
    Manual,
    Mention,
    ReviewRequested,
    SecurityAlert,
    StateChange,
    Subscribed,
    TeamMention,
}

#[derive(Serialize, Deserialize, Debug)]
struct DetailItem {
    html_url: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Notification {
    id: String,
    reason: Reason,
    repository: Repository,
    subject: Subject,
    updated_at: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Repository {
    id: i64,
    name: String,
    full_name: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Subject {
    title: String,
    url: String,
    latest_comment_url: Option<String>,
    #[serde(rename = "type")]
    subject_type: String,
}

impl Notifier {
    pub fn new(token: String) -> Self {
        Notifier {
            token,
            client: Client::new(),
        }
    }

    pub async fn start(&self) {
        let mut last_updated = read_last_updated();
        info!("Notifier started. Last updated date: {}", last_updated);

        loop {
            let update_time = Utc::now();
            match self.fetch_github_notifications().await {
                Ok(notifications) => {
                    let mut handles = Vec::new();
                    for notification in notifications {
                        let updated_at = DateTime::parse_from_rfc3339(&notification.updated_at)
                            .unwrap()
                            .with_timezone(&Utc);

                        if updated_at > last_updated {
                            let notifier_clone = self.clone();
                            let handle = tokio::spawn(async move {
                                notifier_clone.handle_notification(notification).await
                            });
                            handles.push(handle)
                        }
                    }

                    for handle in handles {
                        handle.await.unwrap();
                    }
                }
                Err(e) => error!("Error fetching notifications: {}", e),
            }

            last_updated = update_time;
            write_last_updated(last_updated);

            sleep(Duration::from_secs(30)).await;
        }
    }

    async fn fetch_github_notifications(&self) -> Result<Vec<Notification>, reqwest::Error> {
        let res = self
            .client
            .get(GITHUB_API)
            .header("Authorization", format!("token {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "request")
            .send()
            .await?
            .error_for_status()?
            .json::<Vec<Notification>>()
            .await?;

        Ok(res)
    }

    async fn handle_notification(&self, notification: Notification) {
        let (tx, rx) = mpsc::channel();

        debug!("Notifying about '{}' ('{}')", notification.id, notification.subject.title);

        // Display the notification
        SystemNotification::new()
            .summary(&notification.repository.full_name)
            .appname("GitHub")
            .body(&format!(
                "{} ({}/{:?})",
                &notification.subject.title, notification.subject.subject_type, notification.reason
            ))
            .action("default", "Open")
            .show()
            .unwrap()
            .wait_for_action(move |action| {
                if action == "default" {
                    let tx_clone = tx.clone();
                    tx_clone.send(notification.subject).unwrap();
                }
            });

        // Wait for action and handle it
        if let Ok(subject) = rx.recv() {
            self.open_browser(subject).await;
        }
    }

    async fn open_browser(&self, subject: Subject) {
        let url = if let Some(url) = subject.latest_comment_url {
            url
        } else {
            subject.url
        };

        trace!("Notify URL for '{}': {}", subject.title, url);

        let res = self
            .client
            .get(url)
            .header("Authorization", format!("token {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "request")
            .send()
            .await
            .unwrap()
            .json::<DetailItem>()
            .await
            .unwrap()
            .html_url;

        tokio::task::spawn_blocking(move || {
            Command::new("xdg-open")
                .arg(res)
                .spawn()
                .expect("Failed to open URL")
        })
        .await
        .unwrap();
    }
}

fn write_last_updated(datetime: DateTime<Utc>) {
    if let Some(proj_dirs) =
        ProjectDirs::from("com.github", "lfrancke", "gh-notifier", ambient_authority())
    {
        let cache_dir = proj_dirs.cache_dir().unwrap();
        let mut state_file = cache_dir
            .open_with(
                LAST_UPDATED_STATE_FILE,
                OpenOptions::new().create(true).write(true),
            )
            .unwrap()
            .into_std();
        write!(state_file, "{}", datetime.to_rfc3339()).expect("Failed to write to file");
    }
}

fn read_last_updated() -> DateTime<Utc> {
    if let Some(proj_dirs) =
        ProjectDirs::from("com.github", "lfrancke", "gh-notifier", ambient_authority())
    {
        let cache_dir = proj_dirs.cache_dir().unwrap();
        if let Ok(contents) = cache_dir.read_to_string(LAST_UPDATED_STATE_FILE) {
            return if let Ok(datetime) = DateTime::parse_from_rfc3339(contents.trim()) {
                datetime.with_timezone(&Utc)
            } else {
                Utc::now()
            };
        }
    }

    Utc::now()
}

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(EnvFilter::new("gh-notifier=trace,info"))
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let token = env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN not set");
    let notifier = Notifier::new(token);
    notifier.start().await;
}
