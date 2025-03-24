mod collab_notification;
pub mod incoming_call_notification;

#[cfg(feature = "stories")]
mod stories;

use gpui::App;
use notifications::notification_window;
use std::sync::Arc;
use workspace::AppState;

#[cfg(feature = "stories")]
pub use stories::*;

pub fn init(app_state: &Arc<AppState>, cx: &mut App) {
    incoming_call_notification::init(app_state, cx);
    notification_window::init(app_state, cx);
}
