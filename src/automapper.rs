// SPDX-License-Identifier: MPL-2.0
use color_eyre::eyre::{Context, Result, eyre};
use dmm_tools::dmm::Map;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct RawAutomapperConfig {
	#[serde(default)]
	templates: toml::Table,
}

#[derive(Debug, Deserialize)]
struct RawTemplate {
	map_files: Vec<PathBuf>,
	directory: PathBuf,
	required_map: String,
	coordinates: [i32; 3],
}

pub struct AutomapperConfig {
	base_dir: PathBuf,
	templates: Vec<(String, RawTemplate)>,
}

pub struct AutomapperTemplate {
	pub name: String,
	pub map: Map,
	pub x: usize,
	pub y: usize,
	pub z: usize,
}

impl AutomapperConfig {
	pub fn from_file(path: &Path, game_path: &Path) -> Result<Self> {
		let source = std::fs::read_to_string(path)
			.wrap_err_with(|| format!("failed to read automapper config {}", path.display()))?;
		let raw = toml::from_str::<RawAutomapperConfig>(&source)
			.wrap_err_with(|| format!("failed to parse automapper config {}", path.display()))?;
		let mut templates = Vec::with_capacity(raw.templates.len());
		for (name, value) in raw.templates {
			let template = value
				.try_into::<RawTemplate>()
				.wrap_err_with(|| format!("failed to parse automapper template {name}"))?;
			let [x, y, z] = template.coordinates;
			if x < 1 || y < 1 || z < 1 {
				return Err(eyre!(
					"automapper template {name} has non-positive coordinates [{x}, {y}, {z}]"
				));
			}
			if template.map_files.is_empty() {
				return Err(eyre!("automapper template {name} has no map files"));
			}
			templates.push((name, template));
		}
		Ok(Self {
			base_dir: game_path.to_owned(),
			templates,
		})
	}

	pub fn templates_for(
		&self,
		map_name: &str,
		base_dimensions: (usize, usize, usize),
	) -> Result<Vec<AutomapperTemplate>> {
		let mut selected = Vec::new();
		for (name, template) in &self.templates {
			if template.required_map == "builtin" || template.required_map != map_name {
				continue;
			}
			let map_file = &template.map_files[0];
			let map_path = self.base_dir.join(&template.directory).join(map_file);
			let map = Map::from_file(&map_path).wrap_err_with(|| {
				format!(
					"failed to load automapper template {name} from {}",
					map_path.display()
				)
			})?;
			let (base_x, base_y, base_z) = base_dimensions;
			let (template_x, template_y, template_z) = map.dim_xyz();
			let [x, y, z] = template.coordinates;
			let x = x as usize;
			let y = y as usize;
			let z = z as usize;
			if x + template_x - 1 > base_x
				|| y + template_y - 1 > base_y
				|| z + template_z - 1 > base_z
			{
				return Err(eyre!(
					"automapper template {name} at [{x}, {y}, {z}] with size [{template_x}, \
					 {template_y}, {template_z}] does not fit in map {map_name} with size \
					 [{base_x}, {base_y}, {base_z}]"
				));
			}
			selected.push(AutomapperTemplate {
				name: name.clone(),
				map,
				x,
				y,
				z,
			});
		}
		Ok(selected)
	}
}
