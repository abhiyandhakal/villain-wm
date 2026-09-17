use std::{
    env,
    ffi::OsStr,
    fmt,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
};

use crate::{Request, Response};

#[derive(Debug)]
pub enum Error {
    MissingEnvironment(&'static str),
    Io(std::io::Error),
    Json(serde_json::Error),
    Disconnected,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEnvironment(name) => write!(formatter, "{name} is not set"),
            Self::Io(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
            Self::Disconnected => write!(formatter, "Villain closed the IPC connection"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub fn socket_path() -> Result<PathBuf, Error> {
    if let Some(path) = env::var_os("VILLAIN_SOCKET") {
        return Ok(path.into());
    }
    let runtime =
        env::var_os("XDG_RUNTIME_DIR").ok_or(Error::MissingEnvironment("XDG_RUNTIME_DIR"))?;
    let display =
        env::var_os("WAYLAND_DISPLAY").ok_or(Error::MissingEnvironment("WAYLAND_DISPLAY"))?;
    Ok(socket_path_for_display(runtime.as_ref(), display.as_ref()))
}

pub fn socket_path_for_display(runtime: &OsStr, display: &OsStr) -> PathBuf {
    let display = display.to_string_lossy().replace(['/', '\\'], "_");
    PathBuf::from(runtime).join(format!("villain-{display}.sock"))
}

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    pub fn connect() -> Result<Self, Error> {
        let stream = UnixStream::connect(socket_path()?)?;
        Ok(Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
        })
    }

    pub fn request(&mut self, request: &Request) -> Result<Response, Error> {
        serde_json::to_writer(&mut self.writer, request)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;

        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(Error::Disconnected);
        }
        Ok(serde_json::from_str(&line)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_is_scoped_to_wayland_display() {
        assert_eq!(
            socket_path_for_display(OsStr::new("/run/user/1000"), OsStr::new("wayland-2")),
            PathBuf::from("/run/user/1000/villain-wayland-2.sock")
        );
    }

    #[test]
    fn socket_display_cannot_escape_runtime_directory() {
        assert_eq!(
            socket_path_for_display(OsStr::new("/tmp/runtime"), OsStr::new("../wayland-2")),
            PathBuf::from("/tmp/runtime/villain-.._wayland-2.sock")
        );
    }
}
