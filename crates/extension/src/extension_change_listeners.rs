use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use fs::Fs;
use gpui::{AppContext, Global, ReadGlobal, SharedString, Task};
use parking_lot::RwLock;

use crate::Extension;

pub trait OnThemeExtensionChange: Send + Sync + 'static {
    fn list_theme_names(&self, theme_path: PathBuf, fs: Arc<dyn Fs>) -> Task<Result<Vec<String>>>;

    fn remove_user_themes(&self, themes: Vec<SharedString>);

    fn load_user_theme(&self, theme_path: PathBuf, fs: Arc<dyn Fs>) -> Task<Result<()>>;

    fn reload_current_theme(&self, cx: &mut AppContext);
}

pub trait OnLanguageServerExtensionChange: Send + Sync + 'static {}

pub trait OnContextServerExtensionChange: Send + Sync + 'static {
    fn register(&self, extension: Arc<dyn Extension>, server_id: Arc<str>, cx: &mut AppContext);
}

pub trait OnIndexedDocsProviderExtensionChange: Send + Sync + 'static {
    fn register(&self, extension: Arc<dyn Extension>, provider_id: Arc<str>);
}

#[derive(Default)]
struct GlobalExtensionChangeListeners(Arc<ExtensionChangeListeners>);

impl Global for GlobalExtensionChangeListeners {}

#[derive(Default)]
pub struct ExtensionChangeListeners {
    theme_listener: RwLock<Option<Arc<dyn OnThemeExtensionChange>>>,
    context_server_listener: RwLock<Option<Arc<dyn OnContextServerExtensionChange>>>,
    indexed_docs_provider_listener: RwLock<Option<Arc<dyn OnIndexedDocsProviderExtensionChange>>>,
}

impl ExtensionChangeListeners {
    /// Returns the global [`ExtensionChangeListeners`].
    pub fn global(cx: &AppContext) -> Arc<Self> {
        GlobalExtensionChangeListeners::global(cx).0.clone()
    }

    /// Returns the global [`ExtensionChangeListeners`].
    ///
    /// Inserts a default [`ExtensionChangeListeners`] if one does not yet exist.
    pub fn default_global(cx: &mut AppContext) -> Arc<Self> {
        cx.default_global::<GlobalExtensionChangeListeners>()
            .0
            .clone()
    }

    pub fn new() -> Self {
        Self {
            theme_listener: RwLock::default(),
            context_server_listener: RwLock::default(),
            indexed_docs_provider_listener: RwLock::default(),
        }
    }

    pub fn theme_listener(&self) -> Option<Arc<dyn OnThemeExtensionChange>> {
        self.theme_listener.read().clone()
    }

    pub fn register_theme_listener(
        &self,
        listener: impl OnThemeExtensionChange + Send + Sync + 'static,
    ) {
        self.theme_listener.write().replace(Arc::new(listener));
    }

    pub fn context_server_listener(&self) -> Option<Arc<dyn OnContextServerExtensionChange>> {
        self.context_server_listener.read().clone()
    }

    pub fn register_context_server_listener(
        &self,
        listener: impl OnContextServerExtensionChange + Send + Sync + 'static,
    ) {
        self.context_server_listener
            .write()
            .replace(Arc::new(listener));
    }

    pub fn indexed_docs_provider_listener(
        &self,
    ) -> Option<Arc<dyn OnIndexedDocsProviderExtensionChange>> {
        self.indexed_docs_provider_listener.read().clone()
    }

    pub fn register_indexed_docs_provider_listener(
        &self,
        listener: impl OnIndexedDocsProviderExtensionChange + Send + Sync + 'static,
    ) {
        self.indexed_docs_provider_listener
            .write()
            .replace(Arc::new(listener));
    }
}