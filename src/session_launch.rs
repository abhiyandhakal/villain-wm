//! Session-local compatibility launchers for applications that ignore the
//! standard Wayland session environment when choosing their display backend.

use std::{
    env, fs, io,
    os::{unix::fs::PermissionsExt, unix::process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
};

const SHIM_DIR_ENV: &str = "VILLAIN_SESSION_SHIM_DIR";
const ORIGINAL_PATH_ENV: &str = "VILLAIN_ORIGINAL_PATH";
const WAYLAND_SHIMS: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "chatgpt",
];

pub fn is_shim_invocation() -> bool {
    env::var_os(SHIM_DIR_ENV).is_some()
        && invoked_name().is_some_and(|name| WAYLAND_SHIMS.contains(&name.as_str()))
}

pub fn exec_shim() -> io::Result<()> {
    let name = invoked_name()
        .filter(|name| WAYLAND_SHIMS.contains(&name.as_str()))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unknown session launcher"))?;
    let search_path = env::var_os(ORIGINAL_PATH_ENV)
        .or_else(|| env::var_os("PATH"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is not set"))?;
    let target = find_executable(&name, &search_path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("could not find the real {name} executable"),
        )
    })?;
    let arguments = wayland_arguments(env::args_os().skip(1));
    let error = Command::new(target).args(arguments).exec();
    Err(error)
}

pub fn install() -> io::Result<()> {
    let directory = shim_directory()?;
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    let executable = env::current_exe()?;

    for name in WAYLAND_SHIMS {
        let link = directory.join(name);
        match fs::symlink_metadata(&link) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if fs::read_link(&link)? == executable {
                    continue;
                }
                fs::remove_file(&link)?;
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("refusing to replace non-symlink {}", link.display()),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        std::os::unix::fs::symlink(&executable, link)?;
    }
    Ok(())
}

pub fn environment() -> Option<[(String, String); 3]> {
    let directory = shim_directory().ok()?;
    let original = env::var_os(ORIGINAL_PATH_ENV).or_else(|| env::var_os("PATH"))?;
    let path =
        env::join_paths(std::iter::once(directory.clone()).chain(env::split_paths(&original)))
            .ok()?;
    Some([
        ("PATH".into(), path.to_string_lossy().into_owned()),
        (
            ORIGINAL_PATH_ENV.into(),
            original.to_string_lossy().into_owned(),
        ),
        (
            SHIM_DIR_ENV.into(),
            directory.to_string_lossy().into_owned(),
        ),
    ])
}

fn shim_directory() -> io::Result<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|path| path.join("villain-session-bin"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))
}

fn invoked_name() -> Option<String> {
    env::args_os()
        .next()
        .and_then(|path| Path::new(&path).file_name().map(|name| name.to_owned()))
        .and_then(|name| name.into_string().ok())
}

fn find_executable(name: &str, search_path: &std::ffi::OsStr) -> Option<PathBuf> {
    env::split_paths(search_path)
        .map(|directory| directory.join(name))
        .find(|candidate| {
            fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

fn wayland_arguments(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Vec<std::ffi::OsString> {
    let mut arguments: Vec<_> = arguments.into_iter().collect();
    let explicit = arguments.iter().any(|argument| {
        argument == "--ozone-platform"
            || argument.to_string_lossy().starts_with("--ozone-platform=")
    });
    if !explicit {
        arguments.insert(0, "--ozone-platform=wayland".into());
    }
    arguments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_wayland_to_ordinary_launches() {
        assert_eq!(
            wayland_arguments(["--incognito".into()]),
            ["--ozone-platform=wayland", "--incognito"]
        );
    }

    #[test]
    fn preserves_an_explicit_platform_choice() {
        assert_eq!(
            wayland_arguments(["--ozone-platform=x11".into()]),
            ["--ozone-platform=x11"]
        );
    }
}
