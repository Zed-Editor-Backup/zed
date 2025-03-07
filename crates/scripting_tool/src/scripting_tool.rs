mod session;

pub use session::*;

use assistant_tool::{Tool, ToolRegistry};
use gpui::{App, AppContext as _, Task, WeakEntity, Window};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;
use workspace::Workspace;

pub fn init(cx: &App) {
    let registry = ToolRegistry::global(cx);
    registry.register_tool(ScriptingTool);
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ScriptingToolInput {
    lua_script: String,
}

struct ScriptingTool;

impl Tool for ScriptingTool {
    fn name(&self) -> String {
        "lua-interpreter".into()
    }

    fn description(&self) -> String {
        include_str!("scripting_tool_description.txt").into()
    }

    fn input_schema(&self) -> serde_json::Value {
        let schema = schemars::schema_for!(ScriptingToolInput);
        serde_json::to_value(&schema).unwrap()
    }

    fn run(
        self: Arc<Self>,
        input: serde_json::Value,
        workspace: WeakEntity<Workspace>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<String>> {
        let input = match serde_json::from_value::<ScriptingToolInput>(input) {
            Err(err) => return Task::ready(Err(err.into())),
            Ok(input) => input,
        };
        let Ok(project) = workspace.read_with(cx, |workspace, _cx| workspace.project().clone())
        else {
            return Task::ready(Err(anyhow::anyhow!("No project found")));
        };

        let session = cx.new(|cx| ScriptSession::new(project, cx));
        let lua_script = input.lua_script;
        let script = session.update(cx, |session, cx| session.run_script(lua_script, cx));
        cx.spawn(|_cx| async move {
            let output = script.await?.stdout;
            drop(session);
            Ok(format!("The script output the following:\n{output}"))
        })
    }
}
