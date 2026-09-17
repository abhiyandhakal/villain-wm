//! Shared, compositor-independent Villain IPC protocol and client.

mod client;

use serde::{Deserialize, Serialize};

pub use client::{Client, Error as ClientError, socket_path, socket_path_for_display};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WindowId(pub u64);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WindowInfo {
    pub id: WindowId,
    pub title: String,
    pub app_id: String,
    /// Human-facing, one-based workspace number.
    pub workspace: usize,
    pub minimized: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkspaceInfo {
    /// Human-facing, one-based workspace number.
    pub workspace: usize,
    pub active: bool,
    pub window_count: usize,
    pub visible_window_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum DispatchRequest {
    CloseFocused,
    MinimizeFocused,
    RestoreLastMinimized,
    FocusWorkspace { workspace: usize },
    FocusWindow { window: WindowId },
    RestoreWindow { window: WindowId },
    Spawn { argv: Vec<String> },
    Quit,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "query", rename_all = "kebab-case")]
pub enum Query {
    Windows,
    Workspaces,
    ActiveWindow,
    ActiveWorkspace,
    Version,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum Request {
    Dispatch(DispatchRequest),
    Query(Query),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum Response {
    Ok,
    Windows(Vec<WindowInfo>),
    Workspaces(Vec<WorkspaceInfo>),
    ActiveWindow(Option<WindowInfo>),
    ActiveWorkspace(usize),
    Version { protocol: u32, villain: String },
    Error { message: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "snake_case")]
pub enum Event {
    WindowOpened(WindowInfo),
    WindowClosed(WindowId),
    WindowFocused(Option<WindowId>),
    WindowMinimized(WindowId),
    WindowRestored(WindowId),
    WorkspaceChanged(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_one_json_line() {
        let request = Request::Dispatch(DispatchRequest::FocusWorkspace { workspace: 2 });
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"dispatch","payload":{"action":"focus-workspace","workspace":2}}"#
        );
        assert_eq!(serde_json::from_str::<Request>(&encoded).unwrap(), request);
    }
}
