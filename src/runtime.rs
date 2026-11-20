//! Settings that can change while the server runs.
//!
//! They are read from the TOML file at `CONFIG_FILE`, if set, and reloaded on SIGHUP or when
//! the file's contents change. An invalid file is rejected and the previous settings are kept.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use image::ImageFormat;
use serde::Deserialize;
use tokio::sync::{AcquireError, OwnedSemaphorePermit, Semaphore};
use tracing::{error, info, warn};
use tracing_subscriber::{reload, EnvFilter, Registry};

use crate::engine::OutputConfig;

/// How often the config file is checked for changes.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    pub processing: ProcessingSettings,
    pub output: OutputSettings,
    pub log: LogSettings,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ProcessingSettings {
    /// Memory budget for concurrent image processing, in MiB
    pub memory_mb: u32,
    /// Concurrent processing jobs, defaults to the CPU count
    pub max_workers: Option<usize>,
    /// How long an upload may wait for processing capacity before getting a 503
    pub queue_timeout_secs: u64,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct OutputSettings {
    /// Width / height to center-crop to, none to keep the original ratio
    pub aspect_ratio: Option<f64>,
    /// Width to downscale to, none to keep the original size
    pub width: Option<u32>,
    pub format: OutputFormat,
    /// Lossy WebP and JPEG quality, 0 to 100
    pub quality: f32,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Webp,
    Png,
    Jpeg,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LogSettings {
    /// `RUST_LOG`-style filter, defaults to `RUST_LOG` or `info`
    pub level: String,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self { memory_mb: 1024, max_workers: None, queue_timeout_secs: 10 }
    }
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self { aspect_ratio: Some(1.0), width: Some(1024), format: OutputFormat::Webp, quality: 70. }
    }
}

impl Default for LogSettings {
    fn default() -> Self {
        Self { level: std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()) }
    }
}

impl RuntimeConfig {
    fn parse(contents: &str) -> Result<Self> {
        let config: Self = toml::from_str(contents)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.processing.memory_mb == 0 {
            bail!("processing.memory_mb must be positive");
        }
        if self.processing.max_workers == Some(0) {
            bail!("processing.max_workers must be positive");
        }
        // Also rejects NaN; outside this range crops would leave only a sliver of the image
        if self.output.aspect_ratio.is_some_and(|r| !(1. / 16. ..=16.).contains(&r)) {
            bail!("output.aspect_ratio must be between 1/16 and 16");
        }
        if self.output.width == Some(0) {
            bail!("output.width must be positive");
        }
        if !(0. ..=100.).contains(&self.output.quality) {
            bail!("output.quality must be between 0 and 100");
        }
        self.log_filter()?;
        Ok(())
    }

    fn log_filter(&self) -> Result<EnvFilter> {
        EnvFilter::try_new(&self.log.level).context("log.level is not a valid filter")
    }

    pub fn workers(&self) -> usize {
        self.processing
            .max_workers
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
    }

    pub fn queue_timeout(&self) -> Duration {
        Duration::from_secs(self.processing.queue_timeout_secs)
    }

    pub fn image_format(&self) -> ImageFormat {
        match self.output.format {
            OutputFormat::Webp => ImageFormat::WebP,
            OutputFormat::Png => ImageFormat::Png,
            OutputFormat::Jpeg => ImageFormat::Jpeg,
        }
    }

    pub fn output_config(&self) -> OutputConfig {
        let mut config = OutputConfig::new()
            .format(self.image_format())
            .quality(self.output.quality);
        if let Some(ratio) = self.output.aspect_ratio {
            config = config.aspect_ratio(ratio);
        }
        if let Some(width) = self.output.width {
            config = config.rescale(width);
        }
        config
    }
}

/// A semaphore whose total number of permits can change while permits are held.
pub struct ResizableSemaphore {
    semaphore: Arc<Semaphore>,
    size: Mutex<usize>,
}

impl ResizableSemaphore {
    fn new(size: usize) -> Self {
        Self { semaphore: Arc::new(Semaphore::new(size)), size: Mutex::new(size) }
    }

    pub fn size(&self) -> usize {
        *self.size.lock().unwrap()
    }

    pub async fn acquire(&self, n: u32) -> Result<OwnedSemaphorePermit, AcquireError> {
        self.semaphore.clone().acquire_many_owned(n).await
    }

    /// Growing takes effect immediately. Shrinking removes free permits now and the rest as
    /// running jobs release theirs, so jobs already running are never interrupted.
    fn resize(&self, new_size: usize) {
        let mut size = self.size.lock().unwrap();
        if new_size > *size {
            self.semaphore.add_permits(new_size - *size);
        } else if new_size < *size {
            let missing = (*size - new_size) - self.semaphore.forget_permits(*size - new_size);
            if missing > 0 {
                let semaphore = self.semaphore.clone();
                tokio::spawn(async move {
                    if let Ok(permits) = semaphore.acquire_many_owned(missing as u32).await {
                        permits.forget();
                    }
                });
            }
        }
        *size = new_size;
    }
}

pub type LogHandle = reload::Handle<EnvFilter, Registry>;

pub struct Runtime {
    config: RwLock<Arc<RuntimeConfig>>,
    /// Memory available to image processing, one permit per MiB
    pub memory_budget: ResizableSemaphore,
    /// Concurrent processing jobs
    pub workers: ResizableSemaphore,
    log: LogHandle,
}

impl Runtime {
    /// Loads the initial config, failing if `CONFIG_FILE` is set but missing or invalid.
    pub fn load(path: Option<&Path>, log: LogHandle) -> Result<Self> {
        let config = match path {
            Some(path) => {
                let contents = std::fs::read_to_string(path)
                    .with_context(|| format!("Couldn't read config file {}", path.display()))?;
                RuntimeConfig::parse(&contents)
                    .with_context(|| format!("Invalid config file {}", path.display()))?
            }
            None => RuntimeConfig::default(),
        };

        log.reload(config.log_filter()?)?;

        Ok(Self {
            memory_budget: ResizableSemaphore::new(config.processing.memory_mb as usize),
            workers: ResizableSemaphore::new(config.workers()),
            config: RwLock::new(Arc::new(config)),
            log,
        })
    }

    /// A snapshot of the current settings; use one snapshot per request so it sees consistent values.
    pub fn config(&self) -> Arc<RuntimeConfig> {
        self.config.read().unwrap().clone()
    }

    fn apply(&self, new: RuntimeConfig) -> Result<()> {
        self.log.reload(new.log_filter()?)?;
        self.memory_budget.resize(new.processing.memory_mb as usize);
        self.workers.resize(new.workers());
        info!("Config reloaded: {:?}", new);
        *self.config.write().unwrap() = Arc::new(new);
        Ok(())
    }

    /// Reloads the config file on SIGHUP and whenever its contents change.
    pub fn watch(self: Arc<Self>, path: PathBuf) {
        tokio::spawn(async move {
            let mut last = std::fs::read_to_string(&path).ok();
            let mut poll = tokio::time::interval(POLL_INTERVAL);
            #[cfg(unix)]
            let mut hangup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
                .expect("Couldn't listen for SIGHUP");

            loop {
                #[cfg(unix)]
                let forced = tokio::select! {
                    _ = poll.tick() => false,
                    _ = hangup.recv() => true,
                };
                #[cfg(not(unix))]
                let forced = { poll.tick().await; false };

                let contents = match tokio::fs::read_to_string(&path).await {
                    Ok(contents) => contents,
                    Err(e) => {
                        warn!("Couldn't read config file {}: {}", path.display(), e);
                        continue;
                    }
                };
                if !forced && last.as_deref() == Some(contents.as_str()) {
                    continue;
                }
                last = Some(contents.clone());

                let result = RuntimeConfig::parse(&contents).and_then(|new| {
                    if *self.config() == new {
                        info!("Config file unchanged");
                        return Ok(());
                    }
                    self.apply(new)
                });
                if let Err(e) = result {
                    error!("Rejected config file {}, keeping current settings: {:#}", path.display(), e);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_uses_defaults() {
        assert_eq!(RuntimeConfig::parse("").unwrap(), RuntimeConfig::default());
    }

    #[test]
    fn example_file_matches_defaults() {
        let example = include_str!("../config.example.toml");
        assert_eq!(RuntimeConfig::parse(example).unwrap(), RuntimeConfig::default());
    }

    #[test]
    fn parses_partial_file() {
        let config = RuntimeConfig::parse("[processing]\nmemory_mb = 2048\n[output]\nformat = \"png\"").unwrap();
        assert_eq!(config.processing.memory_mb, 2048);
        assert_eq!(config.image_format(), ImageFormat::Png);
        assert_eq!(config.output.width, Some(1024));
    }

    #[test]
    fn accepts_aspect_ratio_bounds() {
        for ratio in ["0.0625", "16.0"] {
            assert!(RuntimeConfig::parse(&format!("[output]\naspect_ratio = {ratio}")).is_ok(), "rejected: {ratio}");
        }
    }

    #[test]
    fn rejects_invalid_values() {
        for contents in [
            "[processing]\nmemory_mb = 0",
            "[processing]\nmax_workers = 0",
            "[output]\naspect_ratio = -1.0",
            "[output]\naspect_ratio = 16.5",
            "[output]\naspect_ratio = 0.06",
            "[output]\nquality = 150.0",
            "[output]\nformat = \"gif\"",
            "[log]\nlevel = \"no=such=filter\"",
            "[processing]\nmemroy_mb = 10",
        ] {
            assert!(RuntimeConfig::parse(contents).is_err(), "accepted: {contents}");
        }
    }

    #[tokio::test]
    async fn semaphore_resizes_while_permits_are_held() {
        let semaphore = ResizableSemaphore::new(4);
        let held = semaphore.acquire(3).await.unwrap();

        // Only 1 permit is free, so 1 is removed now and 2 more once the holder releases
        semaphore.resize(1);
        assert_eq!(semaphore.semaphore.available_permits(), 0);
        drop(held);
        tokio::task::yield_now().await;
        assert_eq!(semaphore.semaphore.available_permits(), 1);

        semaphore.resize(5);
        assert_eq!(semaphore.semaphore.available_permits(), 5);
    }
}
