//! Atomic loading and validation of Villain's effective configuration.

use std::{
    collections::BTreeMap,
    env, fmt, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::keybinds::KeybindRegistry;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct InputConfig {
    pub tap_to_click: bool,
    pub natural_scroll: bool,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            tap_to_click: true,
            natural_scroll: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub input: InputConfig,
    pub keybinds: KeybindRegistry,
    pub environment: BTreeMap<String, String>,
    path: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BindSpec {
    pub keys: String,
    pub dispatch: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    modkey: Option<String>,
    environment_file: Option<String>,
    input: Option<InputConfig>,
    bind: Option<Vec<BindSpec>>,
}

#[derive(Debug)]
pub struct ConfigError(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ConfigError {}

impl RuntimeConfig {
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(config_path()?)
    }

    pub fn reload(&self) -> Result<Self, ConfigError> {
        Self::load_from(self.path.clone())
    }

    fn load_from(path: PathBuf) -> Result<Self, ConfigError> {
        let file = match fs::read_to_string(&path) {
            Ok(contents) => toml::from_str::<FileConfig>(&contents)
                .map_err(|error| ConfigError(format!("{}: {error}", path.display())))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileConfig::default(),
            Err(error) => {
                return Err(ConfigError(format!(
                    "could not read {}: {error}",
                    path.display()
                )));
            }
        };

        let modkey = file.modkey.as_deref().unwrap_or("Super");
        let keybinds = match file.bind {
            Some(bindings) => KeybindRegistry::from_specs(modkey, &bindings),
            None => KeybindRegistry::defaults(modkey),
        }
        .map_err(ConfigError)?;

        let mut environment = default_environment();
        let default_environment_path = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("environment");
        let (environment_path, required) = match file.environment_file {
            Some(value) => (resolve_path(&value, path.parent())?, true),
            None => (default_environment_path, false),
        };
        match fs::read_to_string(&environment_path) {
            Ok(contents) => parse_environment(&contents, &environment_path, &mut environment)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => {}
            Err(error) => {
                return Err(ConfigError(format!(
                    "could not read {}: {error}",
                    environment_path.display()
                )));
            }
        }

        Ok(Self {
            input: file.input.unwrap_or_default(),
            keybinds,
            environment,
            path,
        })
    }
}

fn config_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = env::var_os("VILLAIN_CONFIG") {
        return Ok(path.into());
    }
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("villain/config.toml"));
    }
    let home = env::var_os("HOME")
        .ok_or_else(|| ConfigError("HOME is not set; cannot locate Villain config".into()))?;
    Ok(PathBuf::from(home).join(".config/villain/config.toml"))
}

fn resolve_path(value: &str, base: Option<&Path>) -> Result<PathBuf, ConfigError> {
    if value == "~" || value.starts_with("~/") {
        let home = env::var_os("HOME")
            .ok_or_else(|| ConfigError("HOME is not set; cannot expand ~".into()))?;
        return Ok(if value == "~" {
            PathBuf::from(home)
        } else {
            PathBuf::from(home).join(&value[2..])
        });
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(base.unwrap_or_else(|| Path::new(".")).join(path))
    }
}

fn default_environment() -> BTreeMap<String, String> {
    let mut environment: BTreeMap<_, _> = [
        ("XDG_SESSION_TYPE", "wayland"),
        ("XDG_CURRENT_DESKTOP", "Villain"),
        ("XDG_SESSION_DESKTOP", "villain"),
        ("ELECTRON_OZONE_PLATFORM_HINT", "wayland"),
        ("MOZ_ENABLE_WAYLAND", "1"),
        ("QT_QPA_PLATFORM", "wayland"),
        ("GDK_BACKEND", "wayland"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    if let Some(session) = crate::session_launch::environment() {
        environment.extend(session);
    }
    environment
}

fn parse_environment(
    contents: &str,
    path: &Path,
    output: &mut BTreeMap<String, String>,
) -> Result<(), ConfigError> {
    for (index, original) in contents.lines().enumerate() {
        let line = original.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            return Err(ConfigError(format!(
                "{}:{}: expected KEY=VALUE",
                path.display(),
                index + 1
            )));
        };
        let key = key.trim();
        if !valid_environment_key(key) {
            return Err(ConfigError(format!(
                "{}:{}: invalid environment variable name {key:?}",
                path.display(),
                index + 1
            )));
        }
        let value = strip_matching_quotes(value.trim()).ok_or_else(|| {
            ConfigError(format!(
                "{}:{}: environment value has unmatched quotes",
                path.display(),
                index + 1
            ))
        })?;
        output.insert(key.into(), value.into());
    }
    Ok(())
}

fn valid_environment_key(key: &str) -> bool {
    let mut characters = key.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn strip_matching_quotes(value: &str) -> Option<&str> {
    let quoted = (value.starts_with('"'), value.ends_with('"'));
    let single_quoted = (value.starts_with('\''), value.ends_with('\''));
    match (quoted, single_quoted) {
        ((true, true), _) | (_, (true, true)) if value.len() >= 2 => {
            Some(&value[1..value.len() - 1])
        }
        ((false, false), (false, false)) => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_wayland_native_and_touchpad_friendly() {
        assert!(InputConfig::default().tap_to_click);
        assert!(InputConfig::default().natural_scroll);
        assert_eq!(default_environment()["XDG_SESSION_TYPE"], "wayland");
        assert_eq!(
            default_environment()["ELECTRON_OZONE_PLATFORM_HINT"],
            "wayland"
        );
    }

    #[test]
    fn environment_parser_is_data_not_shell() {
        let mut values = BTreeMap::new();
        parse_environment(
            "# comment\nexport NAME=Villain\nLITERAL='$HOME and spaces'\n",
            Path::new("environment"),
            &mut values,
        )
        .unwrap();
        assert_eq!(values["NAME"], "Villain");
        assert_eq!(values["LITERAL"], "$HOME and spaces");
    }

    #[test]
    fn invalid_environment_line_rejects_complete_load() {
        let mut values = BTreeMap::new();
        assert!(
            parse_environment("NOT VALID=value", Path::new("environment"), &mut values).is_err()
        );
        assert!(values.is_empty());
    }

    #[test]
    fn shipped_examples_are_valid() {
        let file: FileConfig = toml::from_str(include_str!("../config.example.toml")).unwrap();
        KeybindRegistry::from_specs(
            file.modkey.as_deref().unwrap_or("Super"),
            file.bind.as_deref().unwrap(),
        )
        .unwrap();
        let mut environment = BTreeMap::new();
        parse_environment(
            include_str!("../environment.example"),
            Path::new("environment.example"),
            &mut environment,
        )
        .unwrap();
        assert_eq!(environment["XDG_SESSION_TYPE"], "wayland");
    }

    #[test]
    fn reload_validates_before_returning_replacement() {
        let directory =
            std::env::temp_dir().join(format!("villain-config-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(
            &path,
            "modkey = \"Alt\"\n[input]\ntap_to_click = false\nnatural_scroll = false\n",
        )
        .unwrap();
        let current = RuntimeConfig::load_from(path.clone()).unwrap();
        assert!(!current.input.tap_to_click);
        assert!(!current.input.natural_scroll);

        fs::write(&path, "modkey = \"Invalid\"\n").unwrap();
        assert!(current.reload().is_err());
        assert!(!current.input.tap_to_click);
        fs::remove_dir_all(directory).unwrap();
    }
}
