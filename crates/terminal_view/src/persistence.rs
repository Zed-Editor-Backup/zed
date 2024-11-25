use anyhow::{Context as _, Result};
use std::path::PathBuf;

use db::{
    define_connection, query,
    sqlez::{connection::Connection, statement::Statement},
    sqlez_macros::sql,
};
use workspace::{ItemId, SerializedPaneGroup, WorkspaceDb, WorkspaceId};

pub type GroupId = i64;

define_connection! {
    pub static ref TERMINAL_DB: TerminalDb<WorkspaceDb> =
        &[sql!(
            CREATE TABLE terminals (
                workspace_id INTEGER,
                item_id INTEGER UNIQUE,
                working_directory BLOB,
                PRIMARY KEY(workspace_id, item_id),
                FOREIGN KEY(workspace_id) REFERENCES workspaces(workspace_id)
                ON DELETE CASCADE
            ) STRICT;
        ),
        // Remove the unique constraint on the item_id table
        // SQLite doesn't have a way of doing this automatically, so
        // we have to do this silly copying.
        sql!(
            CREATE TABLE terminals2 (
                workspace_id INTEGER,
                item_id INTEGER,
                working_directory BLOB,
                PRIMARY KEY(workspace_id, item_id),
                FOREIGN KEY(workspace_id) REFERENCES workspaces(workspace_id)
                ON DELETE CASCADE
            ) STRICT;

            INSERT INTO terminals2 (workspace_id, item_id, working_directory)
            SELECT workspace_id, item_id, working_directory FROM terminals;

            DROP TABLE terminals;

            ALTER TABLE terminals2 RENAME TO terminals;
        ),

        // Terminal splits data
        sql!(
            CREATE TABLE terminal_pane_groups(
                group_id INTEGER PRIMARY KEY,
                workspace_id INTEGER,
                item_id INTEGER,
                parent_group_id INTEGER, // NULL indicates that this is a root node
                position INTEGER, // NULL indicates that this is a root node
                axis TEXT NOT NULL, // Enum: 'Vertical' / 'Horizontal'
                flexes TEXT,
                FOREIGN KEY(workspace_id, item_id) REFERENCES terminals(workspace_id, item_id)
                ON DELETE CASCADE,
                FOREIGN KEY(parent_group_id) REFERENCES terminal_pane_groups(group_id) ON DELETE CASCADE
            ) STRICT;
        )];
}

impl TerminalDb {
    query! {
       pub async fn update_workspace_id(
            new_id: WorkspaceId,
            old_id: WorkspaceId,
            item_id: ItemId
        ) -> Result<()> {
            UPDATE terminals
            SET workspace_id = ?
            WHERE workspace_id = ? AND item_id = ?;

            UPDATE terminal_pane_groups
            SET workspace_id = ?
            WHERE workspace_id = ? AND item_id = ?
        }
    }

    query! {
        pub async fn save_working_directory(
            item_id: ItemId,
            workspace_id: WorkspaceId,
            working_directory: PathBuf
        ) -> Result<()> {
            INSERT OR REPLACE INTO terminals(item_id, workspace_id, working_directory)
            VALUES (?, ?, ?);

            UPDATE terminal_pane_groups
            SET workspace_id = ?
            WHERE workspace_id = ? AND item_id = ?
        }
    }

    query! {
        pub fn get_working_directory(item_id: ItemId, workspace_id: WorkspaceId) -> Result<Option<PathBuf>> {
            SELECT working_directory
            FROM terminals
            WHERE item_id = ? AND workspace_id = ?
        }
    }

    pub async fn delete_unloaded_items(
        &self,
        workspace: WorkspaceId,
        alive_items: Vec<ItemId>,
    ) -> Result<()> {
        let placeholders = alive_items
            .iter()
            .map(|_| "?")
            .collect::<Vec<&str>>()
            .join(", ");

        let query = format!(
            "DELETE FROM terminals WHERE workspace_id = ? AND item_id NOT IN ({placeholders})"
        );

        self.write(move |conn| {
            let mut statement = Statement::prepare(conn, query)?;
            let mut next_index = statement.bind(&workspace, 1)?;
            for id in alive_items {
                next_index = statement.bind(&id, next_index)?;
            }
            statement.exec()
        })
        .await
    }

    pub fn save_pane_group(
        conn: &Connection,
        workspace_id: WorkspaceId,
        pane_group: &SerializedPaneGroup,
        parent: Option<(GroupId, usize)>,
    ) -> Result<()> {
        match dbg!(pane_group) {
            SerializedPaneGroup::Group {
                axis,
                children,
                flexes,
            } => {
                let (parent_id, position) = parent.unzip();

                let flex_string = flexes
                    .as_ref()
                    .map(|flexes| serde_json::json!(flexes).to_string());

                let group_id = conn.select_row_bound::<_, i64>(sql!(
                    INSERT INTO terminal_pane_groups(
                        workspace_id,
                        parent_group_id,
                        position,
                        axis,
                        flexes
                    )
                    VALUES (?, ?, ?, ?, ?)
                    RETURNING group_id
                ))?((
                    workspace_id,
                    parent_id,
                    position,
                    *axis,
                    flex_string,
                ))?
                .context("Retrieving retrieve group_id from inserted pane_group")?;

                for (position, group) in children.iter().enumerate() {
                    Self::save_pane_group(conn, workspace_id, group, Some((group_id, position)))?
                }

                Ok(())
            }
            SerializedPaneGroup::Pane(pane) => {
                // TODO kb is it the right way? items are stored in the KV store already
                // Self::save_pane(conn, workspace_id, pane, parent)?;
                Ok(())
            }
        }
    }
}
