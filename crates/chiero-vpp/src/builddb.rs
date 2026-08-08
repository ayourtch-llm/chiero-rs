//! Stub — red for 060 contract 1.

use chiero_pp::ConfigId;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranslationUnit {
    pub src: PathBuf,
    pub dir: PathBuf,
    pub object: PathBuf,
    pub args: Vec<String>,
    pub defines: Vec<(String, String)>,
    pub include_paths: Vec<PathBuf>,
    pub config: ConfigId,
}

impl TranslationUnit {
    pub fn pp_config(&self) -> chiero_pp::Config {
        chiero_pp::Config::default()
    }
}

#[derive(Clone, Debug, Default)]
pub struct BuildDb {
    units: Vec<TranslationUnit>,
}

impl BuildDb {
    pub fn parse(_json: &str) -> Result<Self, String> {
        Ok(Self::default())
    }
    pub fn units(&self) -> &[TranslationUnit] {
        &self.units
    }
    pub fn c_units(&self) -> impl Iterator<Item = &TranslationUnit> {
        self.units.iter()
    }
    pub fn units_for(&self, _src: &Path) -> impl Iterator<Item = &TranslationUnit> {
        self.units.iter()
    }
    pub fn distinct_configs(&self) -> usize {
        0
    }
}
