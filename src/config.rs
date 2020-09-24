use std::collections::HashMap;
use std::fs::{File, metadata};
use std::ops::Sub;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use directories::ProjectDirs;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Config {
    pub(crate) app: HashMap<String, Rc<AppMapping>>
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AppMapping {
    #[serde(default)]
    pub(crate) mode: RatchetMode,
    pub(crate) mapping: HashMap<Modifier, Rc<ButtonMapping>>,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone, Eq, PartialEq)]
pub(crate) enum RatchetMode {
    Free,
    Ratcheted,
}

impl Default for RatchetMode {
    fn default() -> Self {
        Self::Ratcheted
    }
}

#[derive(Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub(crate) enum Modifier {
    None,
    Shift,
    Alt,
    Ctrl,
}

impl From<u8> for Modifier {
    fn from(v: u8) -> Self {
        if v & 0x44 != 0 {
            Self::Alt
        } else if v & 0x22 != 0 {
            Self::Shift
        } else if v & 0x11 != 0 {
            Self::Ctrl
        } else {
            Self::None
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum Operation {
    #[serde(deserialize_with = "deserialize_string_lowercase")]
    KeyPress(u32, u8),
    Execute(String),
}

fn deserialize_string_lowercase<'de, D>(deserializer: D) -> Result<(u32, u8), D::Error>
    where
        D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?.to_lowercase();
    let mut iter = s.rsplit('+');
    use serde::de::Error;
    if let Some(key) = iter.next() {
        let keycode = crate::keysyms::KEYSYMS.get(key).
            map_or_else(|| Err(Error::custom(format!("Unknown keysym: {}", key))), |v| Ok(*v))?;
        let mut modifiers = 0;
        for modifier in iter {
            match modifier {
                "shift" => modifiers = modifiers | 1,
                "alt" => modifiers = modifiers | 8,
                "ctrl" => modifiers = modifiers | 4,
                _ => {}
            }
        }
        Ok((keycode, modifiers))
    } else {
        Err(Error::custom("Can't parse keysym"))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ButtonMapping {
    #[serde(default)]
    pub(crate) touch: Vec<Operation>,
    #[serde(default)]
    pub(crate) release: Vec<Operation>,
    #[serde(default)]
    pub(crate) click: Vec<Operation>,
    #[serde(default)]
    pub(crate) left: Vec<Operation>,
    #[serde(default)]
    pub(crate) right: Vec<Operation>,
    #[serde(default)]
    pub(crate) left_pressed: Vec<Operation>,
    #[serde(default)]
    pub(crate) right_pressed: Vec<Operation>,
}

//#[derive(Debug)]
pub struct ConfigFile {
    config: Option<Config>,
    path: Option<PathBuf>,
    mtime: SystemTime,
    last_mtime_check: Instant,
    active_app: Option<String>,
    global_conf: Option<Rc<AppMapping>>,
    active_conf: Option<Rc<AppMapping>>,
}

impl ConfigFile {
    pub(crate) fn new() -> ConfigFile {
        let mut conf =
            if let Some(dirs) = ProjectDirs::from("org", "prefiks", "crown-controller") {
                let path = dirs.config_dir().join("config.yaml");
                ConfigFile {
                    path: Some(path),
                    config: None,
                    mtime: SystemTime::now(),
                    last_mtime_check: Instant::now().sub(Duration::from_secs(1000)),
                    active_app: None,
                    global_conf: None,
                    active_conf: None,
                }
            } else {
                ConfigFile {
                    config: None,
                    path: None,
                    mtime: SystemTime::now(),
                    last_mtime_check: Instant::now().sub(Duration::from_secs(1000)),
                    active_app: None,
                    global_conf: None,
                    active_conf: None,
                }
            };
        conf.maybe_load_config();
        conf
    }

    pub fn select_app(&mut self, app: &str) {
        self.active_app = Some(app.to_owned());
        self.maybe_load_config();
        self.update_app_config();
    }

    pub(crate) fn get_mapping_for_modifiers(&mut self, modifiers: Modifier) -> (RatchetMode, Option<Rc<ButtonMapping>>) {
        self.maybe_load_config();
        let (active_conf, global_conf) = (self.active_conf.as_ref(), self.global_conf.as_ref());
        active_conf.
            and_then(|v| v.mapping.get(&modifiers).
                and_then(|v2| Some((v.mode, v2.clone())))).
            or_else(|| global_conf.and_then(|ref v| v.mapping.get(&modifiers).
                and_then(|v2| Some((v.mode, v2.clone()))))).
            map_or_else(
                || global_conf.map_or((RatchetMode::Ratcheted, None), |v| (v.mode, None)),
                |(mode, mapping)| (mode, Some(mapping)))
    }

    fn maybe_load_config(&mut self) {
        if let Some(ref path) = self.path {
            if self.last_mtime_check.elapsed() > Duration::from_secs(1) {
                let mtime = metadata(path)
                    .map_or_else(|_e| SystemTime::now(),
                                 |meta| meta.modified()
                                     .unwrap_or_else(|_e| SystemTime::now()));
                if mtime != self.mtime {
                    if let Ok(config_file) = File::open(path) {
                        match serde_yaml::from_reader::<_, Config>(&config_file) {
                            Ok(config) => {
                                self.global_conf = config.app.get("global").map_or(None, |v| Some(v.clone()));
                                self.config = Some(config);
                                self.update_app_config();
                            }
                            Err(err) => {
                                println!("Can't load config: {:?}", err);
                                self.config = None;
                                self.global_conf = None;
                                self.active_conf = None;
                            }
                        }
                    } else {
                        println!("Can't open config file {:?}", path);
                    }
                    self.mtime = mtime;
                }
                self.last_mtime_check = Instant::now();
            }
        }
    }
    fn update_app_config(&mut self) {
        if let Some(ref conf) = self.config {
            if let Some(app) = &self.active_app {
                self.active_conf = conf.app.get(app).map_or(None, |v| Some(v.clone()));
                if self.active_conf.is_none() {
                    if let Some(app) = app.rsplit("/").next() {
                        self.active_conf = conf.app.get(app).map_or(None, |v| Some(v.clone()));
                    }
                }
            }
        }
    }
}
