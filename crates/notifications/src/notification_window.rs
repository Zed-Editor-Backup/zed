use call::{room, ActiveCall};
use client::User;
use collections::HashMap;
use gpui::{App, Pixels, PlatformDisplay, Point, Bounds, Size, WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind, WindowOptions, img, AnyElement, SharedUri, Window};
use release_channel::ReleaseChannel;
use smallvec::SmallVec;
use std::rc::Rc;
use std::sync::{Arc, Weak};
use theme;
use ui::{prelude::*, Button, Label, h_flex, v_flex, px};
use util::ResultExt;
use workspace::AppState;

#[derive(IntoElement)]
struct CollabNotification {
    avatar_uri: SharedUri,
    accept_button: Button,
    dismiss_button: Button,
    children: SmallVec<[AnyElement; 2]>,
}

impl CollabNotification {
    fn new(
        avatar_uri: impl Into<SharedUri>,
        accept_button: Button,
        dismiss_button: Button,
    ) -> Self {
        Self {
            avatar_uri: avatar_uri.into(),
            accept_button,
            dismiss_button,
            children: SmallVec::new(),
        }
    }
}

impl ParentElement for CollabNotification {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl RenderOnce for CollabNotification {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .text_ui(cx)
            .justify_between()
            .size_full()
            .overflow_hidden()
            .elevation_3(cx)
            .p_2()
            .gap_2()
            .child(img(self.avatar_uri).w_12().h_12().rounded_full())
            .child(v_flex().overflow_hidden().children(self.children))
            .child(
                v_flex()
                    .child(self.accept_button)
                    .child(self.dismiss_button),
            )
    }
}

fn notification_window_options(
    screen: Rc<dyn PlatformDisplay>,
    size: Size<Pixels>,
    cx: &App,
) -> WindowOptions {
    let notification_margin_width = px(16.);
    let notification_margin_height = px(-48.);

    let bounds = Bounds::<Pixels> {
        origin: screen.bounds().top_right()
            - Point::new(
                size.width + notification_margin_width,
                notification_margin_height,
            ),
        size,
    };

    let app_id = ReleaseChannel::global(cx).app_id();

    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        display_id: Some(screen.id()),
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(app_id.to_owned()),
        window_min_size: None,
        window_decorations: Some(WindowDecorations::Client),
    }
}

pub fn init(app_state: &Arc<AppState>, cx: &mut App) {
    let app_state = Arc::downgrade(app_state);
    let active_call = ActiveCall::global(cx);
    let mut notification_windows = HashMap::default();
    cx.subscribe(&active_call, move |_, event, cx| match event {
        room::Event::RemoteProjectShared {
            owner,
            project_id,
            worktree_root_names,
        } => {
            let window_size = Size {
                width: px(400.),
                height: px(72.),
            };

            for screen in cx.displays() {
                let options = notification_window_options(screen, window_size, cx);
                let Some(window) = cx
                    .open_window(options, |_, cx| {
                        cx.new(|_| {
                            ProjectSharedNotification::new(
                                owner.clone(),
                                *project_id,
                                worktree_root_names.clone(),
                                app_state.clone(),
                            )
                        })
                    })
                    .log_err()
                else {
                    continue;
                };
                notification_windows
                    .entry(*project_id)
                    .or_insert(Vec::new())
                    .push(window);
            }
        }

        room::Event::RemoteProjectUnshared { project_id }
        | room::Event::RemoteProjectJoined { project_id }
        | room::Event::RemoteProjectInvitationDiscarded { project_id } => {
            if let Some(windows) = notification_windows.remove(project_id) {
                for window in windows {
                    window
                        .update(cx, |_, window, _| {
                            window.remove_window();
                        })
                        .ok();
                }
            }
        }

        room::Event::RoomLeft { .. } => {
            for (_, windows) in notification_windows.drain() {
                for window in windows {
                    window
                        .update(cx, |_, window, _| {
                            window.remove_window();
                        })
                        .ok();
                }
            }
        }
        _ => {}
    })
    .detach();
}

pub struct ProjectSharedNotification {
    project_id: u64,
    worktree_root_names: Vec<String>,
    owner: Arc<User>,
    app_state: Weak<AppState>,
}

impl ProjectSharedNotification {
    fn new(
        owner: Arc<User>,
        project_id: u64,
        worktree_root_names: Vec<String>,
        app_state: Weak<AppState>,
    ) -> Self {
        Self {
            project_id,
            worktree_root_names,
            owner,
            app_state,
        }
    }

    fn join(&mut self, cx: &mut Context<Self>) {
        if let Some(app_state) = self.app_state.upgrade() {
            workspace::join_in_room_project(self.project_id, self.owner.id, app_state, cx)
                .detach_and_log_err(cx);
        }
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if let Some(active_room) =
            ActiveCall::global(cx).read_with(cx, |call, _| call.room().cloned())
        {
            active_room.update(cx, |_, cx| {
                cx.emit(room::Event::RemoteProjectInvitationDiscarded {
                    project_id: self.project_id,
                });
            });
        }
    }
}

impl Render for ProjectSharedNotification {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_font = theme::setup_ui_font(window, cx);

        div().size_full().font(ui_font).child(
            CollabNotification::new(
                self.owner.avatar_uri.clone(),
                Button::new("open", "Open").on_click(cx.listener(move |this, _event, _, cx| {
                    this.join(cx);
                })),
                Button::new("dismiss", "Dismiss").on_click(cx.listener(
                    move |this, _event, _, cx| {
                        this.dismiss(cx);
                    },
                )),
            )
            .child(Label::new(self.owner.github_login.clone()))
            .child(Label::new(format!(
                "is sharing a project in Zed{}",
                if self.worktree_root_names.is_empty() {
                    ""
                } else {
                    ":"
                }
            )))
            .children(if self.worktree_root_names.is_empty() {
                None
            } else {
                Some(Label::new(self.worktree_root_names.join(", ")))
            }),
        )
    }
}
