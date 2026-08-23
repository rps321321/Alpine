use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, Webview, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NavigationStatus {
    Opened,
    ApprovalRequired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigationDecision {
    pub(crate) status: NavigationStatus,
    pub(crate) url: String,
    pub(crate) host: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BrowserBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl BrowserBounds {
    fn validate(&self) -> Result<(), String> {
        let values = [self.x, self.y, self.width, self.height];
        if !values.into_iter().all(f64::is_finite)
            || self.x < 0.0
            || self.y < 0.0
            || self.width < 1.0
            || self.height < 1.0
        {
            return Err("the browser surface bounds are invalid".to_owned());
        }
        Ok(())
    }

    fn position(&self) -> LogicalPosition<f64> {
        LogicalPosition::new(self.x, self.y)
    }

    fn size(&self) -> LogicalSize<f64> {
        LogicalSize::new(self.width, self.height)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BrowserNavigationRequest {
    tab_id: String,
    address: String,
    allow_host: bool,
    bounds: BrowserBounds,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum BrowserEvent {
    Page {
        tab_id: String,
        url: String,
        loading: bool,
    },
    Title {
        tab_id: String,
        title: String,
    },
    AccessRequested {
        tab_id: String,
        url: String,
        host: String,
    },
    NewTabRequested {
        tab_id: String,
        url: String,
    },
    Download {
        tab_id: String,
        url: String,
        path: Option<String>,
        state: &'static str,
    },
}

#[derive(Clone, Default)]
pub(crate) struct BrowserRegistry {
    views: Arc<Mutex<HashMap<String, Webview>>>,
    persistent_hosts: Arc<Mutex<HashSet<String>>>,
    session_hosts: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    creation_lock: Arc<Mutex<()>>,
}

impl BrowserRegistry {
    pub(crate) fn new(allowed_hosts: impl IntoIterator<Item = String>) -> Self {
        let registry = Self::default();
        registry.replace_persistent_hosts(allowed_hosts);
        registry
    }

    pub(crate) fn replace_persistent_hosts(&self, allowed_hosts: impl IntoIterator<Item = String>) {
        let next = allowed_hosts
            .into_iter()
            .map(|host| host.trim().to_ascii_lowercase())
            .filter(|host| !host.is_empty())
            .collect();
        *self.persistent_hosts.lock().expect("browser host lock") = next;
    }

    fn policy_for_tab(&self, tab_id: &str) -> BrowserPolicy {
        let mut hosts = self
            .persistent_hosts
            .lock()
            .expect("browser host lock")
            .clone();
        if let Some(session) = self
            .session_hosts
            .lock()
            .expect("browser session host lock")
            .get(tab_id)
        {
            hosts.extend(session.iter().cloned());
        }
        BrowserPolicy::new(hosts)
    }

    fn allow_for_tab(&self, tab_id: &str, host: &str) {
        self.session_hosts
            .lock()
            .expect("browser session host lock")
            .entry(tab_id.to_owned())
            .or_default()
            .insert(host.to_ascii_lowercase());
    }

    fn view(&self, tab_id: &str) -> Option<Webview> {
        self.views
            .lock()
            .expect("browser view lock")
            .get(tab_id)
            .cloned()
    }

    fn insert(&self, tab_id: String, webview: Webview) {
        self.views
            .lock()
            .expect("browser view lock")
            .insert(tab_id, webview);
    }

    fn remove(&self, tab_id: &str) -> Option<Webview> {
        self.session_hosts
            .lock()
            .expect("browser session host lock")
            .remove(tab_id);
        self.views.lock().expect("browser view lock").remove(tab_id)
    }

    fn all_views(&self) -> Vec<(String, Webview)> {
        self.views
            .lock()
            .expect("browser view lock")
            .iter()
            .map(|(id, view)| (id.clone(), view.clone()))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BrowserPolicy {
    allowed_hosts: HashSet<String>,
}

impl BrowserPolicy {
    pub(crate) fn new(allowed_hosts: impl IntoIterator<Item = String>) -> Self {
        Self {
            allowed_hosts: allowed_hosts
                .into_iter()
                .map(|host| host.trim().to_ascii_lowercase())
                .filter(|host| !host.is_empty())
                .collect(),
        }
    }

    pub(crate) fn evaluate(
        &self,
        address: &str,
        allow_host: bool,
    ) -> Result<NavigationDecision, String> {
        let address = address.trim();
        if address.is_empty() {
            return Err("enter a website or local address".to_owned());
        }
        if address.eq_ignore_ascii_case("about:blank") {
            return Ok(NavigationDecision {
                status: NavigationStatus::Opened,
                url: "about:blank".to_owned(),
                host: None,
            });
        }
        let candidate = if address.starts_with("http://") || address.starts_with("https://") {
            address.to_owned()
        } else if address.contains("://") || address.starts_with("javascript:") {
            return Err("the browser accepts only HTTP and HTTPS addresses".to_owned());
        } else if is_probably_local(address) {
            format!("http://{address}")
        } else {
            format!("https://{address}")
        };
        let url = tauri::Url::parse(&candidate)
            .map_err(|error| format!("the browser address is invalid: {error}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("the browser accepts only HTTP and HTTPS addresses".to_owned());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("put credentials in the website, not in its address".to_owned());
        }
        let host = url
            .host_str()
            .ok_or_else(|| "the browser address must include a host".to_owned())?
            .to_ascii_lowercase();
        let status = if is_local_host(&host) || allow_host || self.allowed_hosts.contains(&host) {
            NavigationStatus::Opened
        } else {
            NavigationStatus::ApprovalRequired
        };
        Ok(NavigationDecision {
            status,
            url: url.to_string(),
            host: Some(host),
        })
    }
}

fn is_probably_local(address: &str) -> bool {
    let host = address
        .trim_start_matches('[')
        .split([']', ':', '/'])
        .next()
        .unwrap_or_default();
    is_local_host(host)
}

fn is_local_host(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

fn validate_tab_id(tab_id: &str) -> Result<&str, String> {
    let valid = !tab_id.is_empty()
        && tab_id.len() <= 80
        && tab_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    valid
        .then_some(tab_id)
        .ok_or_else(|| "the browser tab identifier is invalid".to_owned())
}

fn emit(app: &AppHandle, event: BrowserEvent) {
    let _ = app.emit("browser-event", event);
}

fn profile_directory(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("browser-profile"))
        .map_err(|error| format!("failed to resolve the browser profile directory: {error}"))
}

fn build_webview(
    app: &AppHandle,
    registry: &BrowserRegistry,
    tab_id: &str,
    url: tauri::Url,
    bounds: &BrowserBounds,
) -> Result<Webview, String> {
    if let Some(view) = registry.view(tab_id) {
        return Ok(view);
    }
    let _creation = registry
        .creation_lock
        .lock()
        .expect("browser creation lock");
    if let Some(view) = registry.view(tab_id) {
        return Ok(view);
    }
    let window = app
        .get_window("main")
        .ok_or_else(|| "the Alpine window is unavailable".to_owned())?;
    let profile = profile_directory(app)?;
    std::fs::create_dir_all(&profile)
        .map_err(|error| format!("failed to create {}: {error}", profile.display()))?;

    let navigation_app = app.clone();
    let navigation_registry = registry.clone();
    let navigation_tab = tab_id.to_owned();
    let page_app = app.clone();
    let page_tab = tab_id.to_owned();
    let title_app = app.clone();
    let title_tab = tab_id.to_owned();
    let window_app = app.clone();
    let window_tab = tab_id.to_owned();
    let download_app = app.clone();
    let download_tab = tab_id.to_owned();
    let builder = WebviewBuilder::new(tab_id, WebviewUrl::External(url))
        .data_directory(profile)
        .enable_clipboard_access()
        .on_navigation(move |next| {
            let decision = navigation_registry
                .policy_for_tab(&navigation_tab)
                .evaluate(next.as_str(), false);
            match decision {
                Ok(decision) if decision.status == NavigationStatus::Opened => true,
                Ok(decision) => {
                    if let Some(host) = decision.host {
                        emit(
                            &navigation_app,
                            BrowserEvent::AccessRequested {
                                tab_id: navigation_tab.clone(),
                                url: decision.url,
                                host,
                            },
                        );
                    }
                    false
                }
                Err(_) => false,
            }
        })
        .on_page_load(move |_webview, payload| {
            emit(
                &page_app,
                BrowserEvent::Page {
                    tab_id: page_tab.clone(),
                    url: payload.url().to_string(),
                    loading: payload.event() == PageLoadEvent::Started,
                },
            );
        })
        .on_document_title_changed(move |_webview, title| {
            emit(
                &title_app,
                BrowserEvent::Title {
                    tab_id: title_tab.clone(),
                    title,
                },
            );
        })
        .on_new_window(move |next, _features| {
            emit(
                &window_app,
                BrowserEvent::NewTabRequested {
                    tab_id: window_tab.clone(),
                    url: next.to_string(),
                },
            );
            NewWindowResponse::Deny
        })
        .on_download(move |_webview, event| match event {
            DownloadEvent::Requested { url, destination } => {
                if let Some(path) = download_destination(&download_app, &url) {
                    *destination = path.clone();
                    emit(
                        &download_app,
                        BrowserEvent::Download {
                            tab_id: download_tab.clone(),
                            url: url.to_string(),
                            path: Some(path.to_string_lossy().into_owned()),
                            state: "started",
                        },
                    );
                    true
                } else {
                    false
                }
            }
            DownloadEvent::Finished { url, path, success } => {
                emit(
                    &download_app,
                    BrowserEvent::Download {
                        tab_id: download_tab.clone(),
                        url: url.to_string(),
                        path: path.map(|value| value.to_string_lossy().into_owned()),
                        state: if success { "completed" } else { "failed" },
                    },
                );
                true
            }
            _ => true,
        });
    let webview = window
        .add_child(builder, bounds.position(), bounds.size())
        .map_err(|error| format!("failed to create the browser surface: {error}"))?;
    registry.insert(tab_id.to_owned(), webview.clone());
    Ok(webview)
}

fn existing_webview(registry: &BrowserRegistry, tab_id: &str) -> Option<Webview> {
    registry.view(tab_id)
}

fn download_destination(app: &AppHandle, url: &tauri::Url) -> Option<PathBuf> {
    let directory = app.path().download_dir().ok()?;
    let raw = url
        .path_segments()
        .and_then(|segments| segments.filter(|segment| !segment.is_empty()).next_back())
        .unwrap_or("download");
    let filename: String = raw
        .chars()
        .take(120)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let filename = if filename.is_empty() {
        "download"
    } else {
        &filename
    };
    let candidate = directory.join(filename);
    if !candidate.exists() {
        return Some(candidate);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    Some(directory.join(format!("{timestamp}-{filename}")))
}

#[tauri::command]
pub(crate) async fn browser_navigate(
    app: AppHandle,
    registry: State<'_, BrowserRegistry>,
    request: BrowserNavigationRequest,
) -> Result<NavigationDecision, String> {
    validate_tab_id(&request.tab_id)?;
    request.bounds.validate()?;
    let decision = registry
        .policy_for_tab(&request.tab_id)
        .evaluate(&request.address, request.allow_host)?;
    if decision.status == NavigationStatus::ApprovalRequired {
        return Ok(decision);
    }
    if request.allow_host
        && let Some(host) = &decision.host
    {
        registry.allow_for_tab(&request.tab_id, host);
    }
    let url = tauri::Url::parse(&decision.url)
        .map_err(|error| format!("failed to prepare browser navigation: {error}"))?;
    let existing = existing_webview(registry.inner(), &request.tab_id);
    let webview = build_webview(
        &app,
        registry.inner(),
        &request.tab_id,
        url.clone(),
        &request.bounds,
    )?;
    webview
        .set_position(request.bounds.position())
        .and_then(|_| webview.set_size(request.bounds.size()))
        .and_then(|_| webview.show())
        .map_err(|error| format!("failed to open the browser address: {error}"))?;
    if existing.is_some() {
        webview
            .navigate(url)
            .map_err(|error| format!("failed to open the browser address: {error}"))?;
    }
    Ok(decision)
}

#[tauri::command]
pub(crate) fn browser_sync_surface(
    registry: State<'_, BrowserRegistry>,
    tab_id: Option<String>,
    bounds: Option<BrowserBounds>,
) -> Result<(), String> {
    if let Some(tab_id) = &tab_id {
        validate_tab_id(tab_id)?;
        bounds
            .as_ref()
            .ok_or_else(|| "browser bounds are required for the active tab".to_owned())?
            .validate()?;
    }
    for (id, view) in registry.all_views() {
        if tab_id.as_deref() == Some(id.as_str()) {
            let bounds = bounds.as_ref().expect("validated browser bounds");
            view.set_position(bounds.position())
                .and_then(|_| view.set_size(bounds.size()))
                .and_then(|_| view.show())
                .map_err(|error| format!("failed to position the browser surface: {error}"))?;
        } else {
            view.hide()
                .map_err(|error| format!("failed to hide an inactive browser tab: {error}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn browser_command(
    registry: State<'_, BrowserRegistry>,
    tab_id: String,
    command: String,
) -> Result<(), String> {
    validate_tab_id(&tab_id)?;
    if command == "close" {
        if let Some(view) = registry.remove(&tab_id) {
            view.close()
                .map_err(|error| format!("failed to close the browser tab: {error}"))?;
        }
        return Ok(());
    }
    let view = registry
        .view(&tab_id)
        .ok_or_else(|| "the browser tab is not open".to_owned())?;
    match command.as_str() {
        "back" => view.eval("history.back()"),
        "forward" => view.eval("history.forward()"),
        "reload" => view.reload(),
        "focus" => view.set_focus(),
        _ => return Err("the browser command is invalid".to_owned()),
    }
    .map_err(|error| format!("failed to control the browser tab: {error}"))
}

#[tauri::command]
pub(crate) fn browser_clear_data(
    app: AppHandle,
    registry: State<'_, BrowserRegistry>,
) -> Result<(), String> {
    let views = registry.all_views();
    for (_, view) in &views {
        view.clear_all_browsing_data()
            .map_err(|error| format!("failed to clear browser data: {error}"))?;
    }
    if views.is_empty() {
        let profile = profile_directory(&app)?;
        if profile.exists() {
            std::fs::remove_dir_all(&profile)
                .map_err(|error| format!("failed to clear {}: {error}", profile.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BrowserPolicy, NavigationDecision, NavigationStatus};

    #[test]
    fn local_addresses_open_without_a_permission_prompt() {
        let policy = BrowserPolicy::new(Vec::new());

        assert_eq!(
            policy.evaluate("127.0.0.1:4173", false).unwrap(),
            NavigationDecision {
                status: NavigationStatus::Opened,
                url: "http://127.0.0.1:4173/".to_owned(),
                host: Some("127.0.0.1".to_owned()),
            }
        );
    }

    #[test]
    fn a_new_external_host_requires_an_explicit_decision() {
        let policy = BrowserPolicy::new(Vec::new());

        assert_eq!(
            policy.evaluate("example.com/docs", false).unwrap(),
            NavigationDecision {
                status: NavigationStatus::ApprovalRequired,
                url: "https://example.com/docs".to_owned(),
                host: Some("example.com".to_owned()),
            }
        );
        assert_eq!(
            policy
                .evaluate("https://example.com/docs", true)
                .unwrap()
                .status,
            NavigationStatus::Opened
        );
    }

    #[test]
    fn configured_hosts_open_but_active_content_and_credentials_are_rejected() {
        let policy = BrowserPolicy::new(["docs.rs".to_owned()]);

        assert_eq!(
            policy
                .evaluate("https://docs.rs/tauri", false)
                .unwrap()
                .status,
            NavigationStatus::Opened
        );
        assert!(policy.evaluate("javascript:alert(1)", true).is_err());
        assert!(
            policy
                .evaluate("https://user:secret@example.com/", true)
                .is_err()
        );
    }
}
