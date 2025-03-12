use std::process::Command;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use assistant_tool::Tool;
use gpui::{App, Entity, Task};
use language_model::LanguageModelRequestMessage;
use project::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BashToolInput {
    /// The bash command to execute as a one-liner.
    command: String,
}

pub struct BashTool;

impl Tool for BashTool {
    fn name(&self) -> String {
        "bash".into()
    }

    fn description(&self) -> String {
        "Executes a bash one-liner and returns the combined output. This tool spawns a bash process, combines stdout and stderr into one interleaved stream, and captures that stream into a string which is returned. Use this tool when you need to run shell commands to get information about the system or process files.".into()
    }

    fn input_schema(&self) -> serde_json::Value {
        let schema = schemars::schema_for!(BashToolInput);
        serde_json::to_value(&schema).unwrap()
    }

    fn run(
        self: Arc<Self>,
        input: serde_json::Value,
        _messages: &[LanguageModelRequestMessage],
        _project: Entity<Project>,
        _cx: &mut App,
    ) -> Task<Result<String>> {
        let input: BashToolInput = match serde_json::from_value(input) {
            Ok(input) => input,
            Err(err) => return Task::ready(Err(anyhow!(err))),
        };

        Task::spawn(async move {
            let output = Command::new("bash")
                .arg("-c")
                .arg(input.command)
                .output()
                .await
                .map_err(|err| anyhow!("Failed to execute bash command: {}", err))?;
            
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            
            let combined_output = if stderr.is_empty() {
                stdout
            } else if stdout.is_empty() {
                stderr
            } else {
                format!("{}\n{}", stdout, stderr)
            };
            
            if !output.status.success() {
                let exit_code = output.status.code().unwrap_or(-1);
                return Ok(format!("Command failed with exit code {}\n{}", exit_code, combined_output));
            }
            
            Ok(combined_output)
        })
    }
}