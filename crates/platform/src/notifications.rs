use anyhow::Result;
use gpui::AppContext;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(not(target_os = "macos"))]
mod other;
#[cfg(not(target_os = "macos"))]
use other as platform;

pub fn show_notification(title: &str, body: &str, cx: &impl AppContext) -> Result<()> {
    platform::show_notification(title, body, cx)
}
