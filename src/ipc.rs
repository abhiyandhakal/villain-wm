//! JSON-Lines IPC server. Socket I/O stays off the compositor thread.

use std::{
    ffi::OsStr,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::FileTypeExt, net::UnixListener},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

use smithay::reexports::calloop::{EventLoop, channel};
use villain_ipc::{PROTOCOL_VERSION, Query, Request, Response, socket_path_for_display};

use crate::{dispatch::Dispatch, state::Villain};

struct Envelope {
    request: Request,
    response: mpsc::Sender<Response>,
}

pub struct IpcServer {
    path: PathBuf,
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        remove_socket(&self.path);
    }
}

pub fn init(
    event_loop: &mut EventLoop<Villain>,
    display: &OsStr,
) -> Result<IpcServer, Box<dyn std::error::Error>> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let path = socket_path_for_display(runtime.as_ref(), display);
    remove_stale_socket(&path)?;
    let listener = UnixListener::bind(&path)?;
    let mut permissions = fs::metadata(&path)?.permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o600);
    fs::set_permissions(&path, permissions)?;

    let (sender, receiver) = channel::channel::<Envelope>();
    event_loop
        .handle()
        .insert_source(receiver, |event, _, state| {
            if let channel::Event::Msg(envelope) = event {
                let response = state.handle_ipc(envelope.request);
                let _ = envelope.response.send(response);
            }
        })?;

    thread::Builder::new()
        .name("villain-ipc".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    break;
                };
                let sender = sender.clone();
                let _ = thread::Builder::new()
                    .name("villain-ipc-client".into())
                    .spawn(move || serve_connection(stream, sender));
            }
        })?;

    tracing::info!(path = %path.display(), "IPC server listening");
    Ok(IpcServer { path })
}

fn serve_connection(mut stream: std::os::unix::net::UnixStream, sender: channel::Sender<Envelope>) {
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader_stream);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                if write_response(
                    &mut stream,
                    &Response::Error {
                        message: format!("invalid request: {error}"),
                    },
                )
                .is_err()
                {
                    return;
                }
                continue;
            }
        };
        let (response_sender, response_receiver) = mpsc::channel();
        if sender
            .send(Envelope {
                request,
                response: response_sender,
            })
            .is_err()
        {
            return;
        }
        let Ok(response) = response_receiver.recv() else {
            return;
        };
        if write_response(&mut stream, &response).is_err() {
            return;
        }
    }
}

fn write_response(
    stream: &mut std::os::unix::net::UnixStream,
    response: &Response,
) -> Result<(), Box<dyn std::error::Error>> {
    serde_json::to_writer(&mut *stream, response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

impl Villain {
    fn handle_ipc(&mut self, request: Request) -> Response {
        match request {
            Request::Dispatch(request) => self
                .dispatch(Dispatch::from(request))
                .map(|()| Response::Ok)
                .unwrap_or_else(|error| Response::Error {
                    message: error.to_string(),
                }),
            Request::Query(query) => match query {
                Query::Windows => Response::Windows(self.window_info()),
                Query::Workspaces => Response::Workspaces(self.workspace_info()),
                Query::ActiveWindow => Response::ActiveWindow(self.active_window_info()),
                Query::ActiveWorkspace => Response::ActiveWorkspace(self.active_workspace + 1),
                Query::Version => Response::Version {
                    protocol: PROTOCOL_VERSION,
                    villain: env!("CARGO_PKG_VERSION").into(),
                },
            },
        }
    }
}

fn remove_stale_socket(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path),
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("refusing to replace non-socket {}", path.display()),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_socket(path: &Path) {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket()) {
        let _ = fs::remove_file(path);
    }
}
