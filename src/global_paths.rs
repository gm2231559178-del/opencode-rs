use std::path::PathBuf;

pub struct GlobalPaths {
    pub home: PathBuf,
    pub data: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub tmp: PathBuf,
    pub bin: PathBuf,
    pub log: PathBuf,
    pub repos: PathBuf,
}

impl GlobalPaths {
    pub fn new() -> Self {
        let home = dirs::home_dir()
            .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/tmp"));

        let xdg_data = std::env::var("XDG_DATA_HOME")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local").join("share").join("opencode"));

        let xdg_config = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config").join("opencode"));

        let xdg_cache = std::env::var("XDG_CACHE_HOME")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".cache").join("opencode"));

        let xdg_state = std::env::var("XDG_STATE_HOME")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local").join("state").join("opencode"));

        let tmp = std::env::temp_dir().join("opencode");

        GlobalPaths {
            home,
            data: xdg_data.clone(),
            config: xdg_config,
            cache: xdg_cache.clone(),
            state: xdg_state,
            tmp: tmp.clone(),
            bin: xdg_cache.join("bin"),
            log: xdg_data.join("log"),
            repos: xdg_data.join("repos"),
        }
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data)?;
        std::fs::create_dir_all(&self.config)?;
        std::fs::create_dir_all(&self.state)?;
        std::fs::create_dir_all(&self.tmp)?;
        std::fs::create_dir_all(&self.log)?;
        std::fs::create_dir_all(&self.bin)?;
        std::fs::create_dir_all(&self.repos)?;
        Ok(())
    }
}

impl Default for GlobalPaths {
    fn default() -> Self {
        Self::new()
    }
}
