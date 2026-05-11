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
	pub categories: Vec<MapCategory>,
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

#[derive(Debug, Clone)]
pub struct ResolvedFlags {
	pub supports_pipes: bool,
	pub render_once: bool,
	pub do_ftl: bool,
}

impl ResolvedFlags {
	pub fn resolve(map: &MapConfig, sub: Option<&MapSubCategory>, cat: &MapCategory) -> Self {
		Self {
			supports_pipes: map
				.supports_pipes
				.or_else(|| sub.and_then(|s| s.supports_pipes))
				.or(cat.supports_pipes)
				.unwrap_or(true),
			render_once: map
				.render_once
				.or_else(|| sub.and_then(|s| s.render_once))
				.or(cat.render_once)
				.unwrap_or(false),
			do_ftl: map
				.do_ftl
				.or_else(|| sub.and_then(|s| s.do_ftl))
				.or(cat.do_ftl)
				.unwrap_or(true),
		}
	}
}

#[derive(Debug, Clone, Deserialize)]
pub struct MapCategory {
	pub name: String,
	#[serde(default)]
	pub maps: Vec<MapConfig>,
	#[serde(default)]
	pub subcategories: Vec<MapSubCategory>,
	pub supports_pipes: Option<bool>,
	pub render_once: Option<bool>,
	pub do_ftl: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MapSubCategory {
	pub name: String,
	#[serde(default)]
	pub maps: Vec<MapConfig>,
	pub supports_pipes: Option<bool>,
	pub render_once: Option<bool>,
	pub do_ftl: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MapConfig {
	pub map_name: String,
	pub dmm_path: PathBuf,
	pub friendly_name: Option<String>,
	pub supports_pipes: Option<bool>,
	pub render_once: Option<bool>,
	pub do_ftl: Option<bool>,
}

impl MapConfig {
	pub fn name(&self) -> &str {
		self.friendly_name.as_ref().unwrap_or(&self.map_name)
	}
}
