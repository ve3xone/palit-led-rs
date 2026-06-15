//! TOML config. Lives next to the exe as `palit-led.toml`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// GPU selection: "all" or list of indices like [0, 1]. Default first GPU.
    pub gpus: GpuSel,
    pub temp: TempCfg,
    pub rainbow: RainbowCfg,
    pub r#static: StaticCfg,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum GpuSel {
    All(String),
    List(Vec<usize>),
}

impl Default for GpuSel {
    fn default() -> Self {
        GpuSel::List(vec![0])
    }
}

impl GpuSel {
    pub fn resolve(&self, count: usize) -> Vec<usize> {
        match self {
            GpuSel::All(s) if s.eq_ignore_ascii_case("all") => (0..count).collect(),
            GpuSel::All(_) => vec![0],
            GpuSel::List(v) => v.iter().copied().filter(|&i| i < count).collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct TempCfg {
    pub interval_ms: u64,
    pub green_max: i32,   // green:  t <= green_max
    pub yellow_max: i32,  // yellow: green_max < t <= yellow_max
    pub orange_max: i32,  // orange: yellow_max < t <= orange_max
    pub red_full: i32,    // smooth: fully red at/after this
    pub smooth: bool,
}

impl Default for TempCfg {
    fn default() -> Self {
        TempCfg {
            interval_ms: 1000,
            green_max: 57,
            yellow_max: 68,
            orange_max: 75,
            red_full: 82,
            smooth: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct RainbowCfg {
    pub interval_ms: u64,
    pub step_deg: f32,
    pub brightness: u8,
}

impl Default for RainbowCfg {
    fn default() -> Self {
        RainbowCfg { interval_ms: 40, step_deg: 2.0, brightness: 100 }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct StaticCfg {
    pub color: String,
    pub brightness: u8,
}

impl Default for StaticCfg {
    fn default() -> Self {
        StaticCfg { color: "00FF00".into(), brightness: 100 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            gpus: GpuSel::default(),
            temp: TempCfg::default(),
            rainbow: RainbowCfg::default(),
            r#static: StaticCfg::default(),
        }
    }
}

pub fn config_path() -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    p.set_file_name("palit-led.toml");
    p
}

pub fn load() -> Config {
    let p = config_path();
    match std::fs::read_to_string(&p) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("config parse error ({e}); using defaults");
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

pub fn write_default() -> std::io::Result<PathBuf> {
    let p = config_path();
    let cfg = Config::default();
    std::fs::write(&p, toml::to_string_pretty(&cfg).unwrap())?;
    Ok(p)
}
