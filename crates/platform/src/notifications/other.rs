use anyhow::Result;
use gpui::AppContext;

pub fn show_notification(_title: &str, _body: &str, _: &impl AppContext) -> Result<()> {
    // TODO: Implement for other platforms
    Ok(())
}
