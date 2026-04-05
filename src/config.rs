// SPDX-License-Identifier: MPL-2.0
use serde::Deserialize;
use std::path::PathBuf;

const fn default_opt_preset() -> u8 {
	2
}

const fn default_true() -> bool {
	true
}

fn default_out_folder() -> PathBuf {
	"out".into()
}

#[derive(Default, Debug, Clone, Deserialize)]
pub struct RenderPassesConfig {
	#[serde(default)]
	pub include: Vec<String>,
	#[serde(default)]
	pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
	pub name: String,
	pub game_path: PathBuf,
	pub dme_name: String,
	pub map_files_path: PathBuf,
	#[serde(default = "default_out_folder")]
	pub out_path: PathBuf,
	#[serde(default = "default_opt_preset")]
	pub optimize_level: u8,
	#[serde(default = "default_true")]
	pub generate_webp: bool,
	#[serde(default)]
	pub render_passes: RenderPassesConfig,
	pub maps: Vec<MapConfig>,
}

impl ServerConfig {
	pub fn optimize_options(&self) -> oxipng::Options {
		oxipng::Options {
			optimize_alpha: true,
			strip: oxipng::StripChunks::Safe,
			..oxipng::Options::from_preset(self.optimize_level)
		}
	}

	pub fn base_map_path(&self) -> PathBuf {
		self.game_path.join(&self.map_files_path)
	}
}

#[derive(Debug, Clone, Deserialize)]
pub struct MapConfig {
	pub map_name: String,
	pub dmm_path: PathBuf,
	pub friendly_name: Option<String>,
}

impl MapConfig {
	pub fn name(&self) -> &str {
		self.friendly_name.as_ref().unwrap_or(&self.map_name)
	}
}
