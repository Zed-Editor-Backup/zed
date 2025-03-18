use crate::Thread;
use anyhow::Result;
use assistant_tool::ActionLog;
use collections::{HashMap, HashSet};
use editor::{Editor, EditorEvent, MultiBuffer};
use gpui::{
    prelude::*, AnyElement, AnyView, App, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, WeakEntity, Window,
};
use language::{Anchor, Capability, OffsetRangeExt};
use multi_buffer::PathKey;
use project::{Project, ProjectPath};
use std::any::{Any, TypeId};
use ui::prelude::*;
use workspace::{
    item::{BreadcrumbText, ItemEvent, TabContentParams},
    searchable::SearchableItemHandle,
    Item, ItemHandle, ItemNavHistory, ToolbarItemLocation, Workspace,
};

pub struct AssistantDiff {
    multibuffer: Entity<MultiBuffer>,
    editor: Entity<Editor>,
    thread: Entity<Thread>,
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    _subscriptions: Vec<Subscription>,
}

impl AssistantDiff {
    pub fn deploy(
        thread: Entity<Thread>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        let existing_diff = workspace.update(cx, |workspace, cx| {
            workspace
                .items_of_type::<AssistantDiff>(cx)
                .find(|diff| diff.read(cx).thread == thread)
        })?;
        if let Some(existing_diff) = existing_diff {
            workspace.update(cx, |workspace, cx| {
                workspace.activate_item(&existing_diff, true, true, window, cx);
            })
        } else {
            let assistant_diff =
                cx.new(|cx| AssistantDiff::new(thread.clone(), workspace.clone(), window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.add_item_to_center(Box::new(assistant_diff), window, cx);
            })
        }
    }

    pub fn new(
        thread: Entity<Thread>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let multibuffer = cx.new(|_| MultiBuffer::new(Capability::ReadWrite));

        let project = thread.read(cx).project().clone();
        let editor = cx.new(|cx| {
            let mut editor =
                Editor::for_multibuffer(multibuffer.clone(), Some(project.clone()), window, cx);
            editor.disable_inline_diagnostics();
            editor.set_expand_all_diff_hunks(cx);
            // todo!()
            // editor.register_addon(...);
            editor
        });

        let action_log = thread.read(cx).action_log().clone();
        let mut this = Self {
            _subscriptions: vec![cx.observe_in(
                &action_log,
                window,
                |this, _action_log, window, cx| this.refresh(window, cx),
            )],
            multibuffer,
            editor,
            thread,
            focus_handle,
            workspace,
        };
        this.refresh(window, cx);
        this
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let thread = self.thread.read(cx);
        let mut paths_to_delete = self.multibuffer.read(cx).paths().collect::<HashSet<_>>();

        for (buffer, tracked) in thread.action_log().read(cx).unreviewed_buffers() {
            let Some(file) = buffer.read(cx).file().cloned() else {
                continue;
            };

            let path_key = PathKey::namespaced("", file.full_path(cx).into());
            paths_to_delete.remove(&path_key);

            let snapshot = buffer.read(cx).snapshot();
            let diff = tracked.diff.read(cx);
            let diff_hunk_ranges = diff
                .hunks_intersecting_range(Anchor::MIN..Anchor::MAX, &snapshot, cx)
                .map(|diff_hunk| diff_hunk.buffer_range.to_point(&snapshot))
                .collect::<Vec<_>>();

            let (was_empty, is_excerpt_newly_added) =
                self.multibuffer.update(cx, |multibuffer, cx| {
                    let was_empty = multibuffer.is_empty();
                    let is_newly_added = multibuffer.set_excerpts_for_path(
                        path_key.clone(),
                        buffer,
                        diff_hunk_ranges,
                        editor::DEFAULT_MULTIBUFFER_CONTEXT,
                        cx,
                    );
                    multibuffer.add_diff(tracked.diff.clone(), cx);
                    (was_empty, is_newly_added)
                });

            self.editor.update(cx, |editor, cx| {
                if was_empty {
                    editor.change_selections(None, window, cx, |selections| {
                        // TODO select the very beginning (possibly inside a deletion)
                        selections.select_ranges([0..0])
                    });
                }
                if is_excerpt_newly_added && !file.disk_state().exists() {
                    editor.fold_buffer(snapshot.text.remote_id(), cx)
                }
            });
        }

        self.multibuffer.update(cx, |multibuffer, cx| {
            for path in paths_to_delete {
                multibuffer.remove_excerpts_for_path(path, cx);
            }
        });

        if self.multibuffer.read(cx).is_empty()
            && self
                .editor
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
        {
            self.focus_handle.focus(window);
        } else if self.focus_handle.is_focused(window) && !self.multibuffer.read(cx).is_empty() {
            self.editor.update(cx, |editor, cx| {
                editor.focus_handle(cx).focus(window);
            });
        }
    }
}

impl EventEmitter<EditorEvent> for AssistantDiff {}

impl Focusable for AssistantDiff {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.multibuffer.read(cx).is_empty() {
            self.focus_handle.clone()
        } else {
            self.editor.focus_handle(cx)
        }
    }
}

impl Item for AssistantDiff {
    type Event = EditorEvent;

    fn tab_icon(&self, _window: &Window, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::Ai).color(Color::Muted))
    }

    fn to_item_events(event: &EditorEvent, f: impl FnMut(ItemEvent)) {
        Editor::to_item_events(event, f)
    }

    fn deactivated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.deactivated(window, cx));
    }

    fn navigate(
        &mut self,
        data: Box<dyn Any>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.editor
            .update(cx, |editor, cx| editor.navigate(data, window, cx))
    }

    fn tab_tooltip_text(&self, _: &App) -> Option<SharedString> {
        Some("Project Diff".into())
    }

    fn tab_content(&self, params: TabContentParams, _window: &Window, _: &App) -> AnyElement {
        Label::new("Uncommitted Changes")
            .color(if params.selected {
                Color::Default
            } else {
                Color::Muted
            })
            .into_any_element()
    }

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("Project Diff Opened")
    }

    fn as_searchable(&self, _: &Entity<Self>) -> Option<Box<dyn SearchableItemHandle>> {
        Some(Box::new(self.editor.clone()))
    }

    fn for_each_project_item(
        &self,
        cx: &App,
        f: &mut dyn FnMut(gpui::EntityId, &dyn project::ProjectItem),
    ) {
        self.editor.for_each_project_item(cx, f)
    }

    fn is_singleton(&self, _: &App) -> bool {
        false
    }

    fn set_nav_history(
        &mut self,
        nav_history: ItemNavHistory,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |editor, _| {
            editor.set_nav_history(Some(nav_history));
        });
    }

    fn clone_on_split(
        &self,
        _workspace_id: Option<workspace::WorkspaceId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Self>>
    where
        Self: Sized,
    {
        Some(cx.new(|cx| Self::new(self.thread.clone(), self.workspace.clone(), window, cx)))
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.multibuffer.read(cx).is_dirty(cx)
    }

    fn has_conflict(&self, cx: &App) -> bool {
        self.multibuffer.read(cx).has_conflict(cx)
    }

    fn can_save(&self, _: &App) -> bool {
        true
    }

    fn save(
        &mut self,
        format: bool,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        self.editor.save(format, project, window, cx)
    }

    fn save_as(
        &mut self,
        _: Entity<Project>,
        _: ProjectPath,
        _window: &mut Window,
        _: &mut Context<Self>,
    ) -> Task<Result<()>> {
        unreachable!()
    }

    fn reload(
        &mut self,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        self.editor.reload(project, window, cx)
    }

    fn act_as_type<'a>(
        &'a self,
        type_id: TypeId,
        self_handle: &'a Entity<Self>,
        _: &'a App,
    ) -> Option<AnyView> {
        if type_id == TypeId::of::<Self>() {
            Some(self_handle.to_any())
        } else if type_id == TypeId::of::<Editor>() {
            Some(self.editor.to_any())
        } else {
            None
        }
    }

    fn breadcrumb_location(&self, _: &App) -> ToolbarItemLocation {
        ToolbarItemLocation::PrimaryLeft
    }

    fn breadcrumbs(&self, theme: &theme::Theme, cx: &App) -> Option<Vec<BreadcrumbText>> {
        self.editor.breadcrumbs(theme, cx)
    }

    fn added_to_workspace(
        &mut self,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |editor, cx| {
            editor.added_to_workspace(workspace, window, cx)
        });
    }
}

impl Render for AssistantDiff {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_empty = self.multibuffer.read(cx).is_empty();
        div()
            .track_focus(&self.focus_handle)
            .key_context(if is_empty {
                "EmptyPane"
            } else {
                "AssistantDiff"
            })
            .bg(cx.theme().colors().editor_background)
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .when(is_empty, |el| el.child("No changes to review"))
            .when(!is_empty, |el| el.child(self.editor.clone()))
    }
}
