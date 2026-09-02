// SPDX-License-Identifier: MPL-2.0
use crate::{
	automapper::{AutomapperConfig, AutomapperTemplate},
	config::{MapConfig, ResolvedFlags, ServerConfig},
	context::DmContext,
	encode::generate_minimap_image,
};
use bumpalo::Bump;
use color_eyre::eyre::{Context, Result, eyre};
use dm::config::MapRenderer;
use dmm_tools::{
	dmi::Rgba8,
	dmm::{self, Map},
	minimap,
	render_passes::RenderPass,
};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::{
	cell::RefCell,
	path::{Path, PathBuf},
	sync::RwLock,
	time::Duration,
};

pub struct RenderPassHolder {
	pub main: Vec<Box<dyn RenderPass>>,
	pub pipes: Vec<Box<dyn RenderPass>>,
}

pub fn create_render_passes(config: &ServerConfig, map_renderer: &MapRenderer) -> RenderPassHolder {
	let main = dmm_tools::render_passes::configure_list(
		map_renderer,
		&config.render_passes.include,
		&config.render_passes.exclude,
	);
	let pipes =
		dmm_tools::render_passes::configure_list(map_renderer, &["only-wires-and-pipes"], &[]);
	RenderPassHolder { main, pipes }
}

pub struct GeneratedMinimap {
	pub map_dir: PathBuf,
	pub name: String,
	pub z: usize,
	pub image: dmm_tools::dmi::Image,
}

#[derive(Clone)]
pub struct MapOutputSpec {
	pub base_map_path: PathBuf,
	pub map_dir: PathBuf,
	pub pipes_dir: Option<PathBuf>,
	pub flags: ResolvedFlags,
	pub automapper: Option<std::sync::Arc<AutomapperConfig>>,
}

thread_local! {
	static BUMP: RefCell<Bump> = RefCell::new(Bump::new());
}

pub struct ProgressState {
	pub mp: MultiProgress,
	pub total: ProgressBar,
}

fn map_bar_style() -> ProgressStyle {
	ProgressStyle::with_template(
		"  {spinner:.cyan} {prefix:.bold.cyan}: {bar:30.cyan/white.dim} {pos}/{len} z-levels",
	)
	.expect("invalid progress style template")
	.progress_chars("=>-")
	.tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", ""])
}

pub fn generate_minimap(
	map_config: &MapConfig,
	dm_context: &DmContext,
	render_passes: &RenderPassHolder,
	config: &ServerConfig,
	optimize_options: &oxipng::Options,
	output: MapOutputSpec,
	progress: &ProgressState,
) -> Result<()> {
	let ProgressState {
		mp: multi_progress,
		total: total_bar,
	} = progress;
	let MapOutputSpec {
		base_map_path,
		map_dir,
		pipes_dir,
		flags,
		automapper,
	} = output;
	use rayon::iter::{IntoParallelIterator, ParallelIterator};

	let map_path = base_map_path.join(&map_config.dmm_path);
	let map = dmm::Map::from_file(&map_path).wrap_err_with(|| {
		format!(
			"failed to load {} from {}",
			map_config.name,
			map_path.display()
		)
	})?;
	let (dim_x, dim_y, dim_z) = map.dim_xyz();
	/* total_bar.println(format!(
		"{}: dim_x={dim_x}, dim_y={dim_y}, dim_z={dim_z}",
		map_config.name
	)); */

	if flags.render_once {
		let all_exist = (1..=dim_z).all(|z| {
			map_dir
				.join(format!("{}-{z}.png", map_config.name))
				.exists()
		});
		if all_exist {
			total_bar.println(format!(
				"{}: skipping (renderOnce, outputs exist)",
				map_config.name
			));
			return Ok(());
		}
	}

	let map_name = map_path
		.file_name()
		.and_then(|name| name.to_str())
		.ok_or_else(|| eyre!("map path has no valid filename: {}", map_path.display()))?;
	let templates = automapper
		.as_ref()
		.map(|config| config.templates_for(map_name, (dim_x, dim_y, dim_z)))
		.transpose()?
		.unwrap_or_default();
	if !templates.is_empty() {
		total_bar.println(format!(
			"{}: applying {} automapper templates",
			map_config.name,
			templates.len()
		));
	}

	let passes_count = if pipes_dir.is_some() { 2u64 } else { 1u64 };
	total_bar.inc_length(dim_z as u64 * passes_count);

	let map_bar =
		multi_progress.insert_before(total_bar, ProgressBar::new(dim_z as u64 * passes_count));
	map_bar.set_style(map_bar_style());
	map_bar.set_prefix(map_config.display_name().to_owned());
	map_bar.enable_steady_tick(Duration::from_millis(100));

	std::fs::create_dir_all(&map_dir)
		.wrap_err_with(|| format!("failed to create map directory {}", map_dir.display()))?;
	let mapinfo = format!("{{\"size\":[{dim_x},{dim_y},{dim_z}]}}");
	std::fs::write(map_dir.join("mapinfo.json"), mapinfo)
		.wrap_err("failed to write mapinfo.json")?;

	(0..dim_z).into_par_iter().for_each(|z| {
		if let Err(err) = generate_for_z(
			&map,
			z,
			map_config,
			dm_context,
			&render_passes.main,
			&templates,
			config,
			optimize_options,
			&map_dir,
			total_bar,
		) {
			total_bar.println(format!(
				"failed to generate minimap for {} (z={}): {err}",
				map_config.name,
				z + 1
			));
		}
		map_bar.inc(1);
	});

	if let Some(ref pipes_dir) = pipes_dir {
		(0..dim_z).into_par_iter().for_each(|z| {
			if let Err(err) = generate_for_z(
				&map,
				z,
				map_config,
				dm_context,
				&render_passes.pipes,
				&templates,
				config,
				optimize_options,
				pipes_dir,
				total_bar,
			) {
				total_bar.println(format!(
					"failed to generate pipes minimap for {} (z={}): {err}",
					map_config.name,
					z + 1
				));
			}
			map_bar.inc(1);
		});
	}

	map_bar.finish_and_clear();
	Ok(())
}

#[allow(clippy::too_many_arguments)]
fn generate_for_z(
	map: &Map,
	z: usize,
	map_config: &MapConfig,
	dm_context: &DmContext,
	render_passes: &[Box<dyn RenderPass>],
	templates: &[AutomapperTemplate],
	config: &ServerConfig,
	optimize_options: &oxipng::Options,
	map_dir: &Path,
	encode_bar: &ProgressBar,
) -> Result<()> {
	BUMP.with_borrow_mut(|bump| {
		let mut image = {
			let errors = RwLock::default();
			let (dim_x, dim_y, _dim_z) = map.dim_xyz();
			let minimap_context = minimap::Context {
				objtree: &dm_context.objtree,
				map,
				level: map.z_level(z),
				min: (0, 0),
				max: (dim_x - 1, dim_y - 1),
				render_passes,
				errors: &errors,
				bump,
				print_errors: false,
			};
			minimap::generate(minimap_context, &dm_context.icon_cache)
				.map_err(|_| eyre!("failed to generate minimap"))?
		};
		bump.reset();

		for template in templates {
			let Some(template_z) = (z + 1).checked_sub(template.z) else {
				continue;
			};
			if template_z >= template.map.dim_z() {
				continue;
			}
			let template_image = {
				let errors = RwLock::default();
				let (template_x, template_y, _template_z) = template.map.dim_xyz();
				let minimap_context = minimap::Context {
					objtree: &dm_context.objtree,
					map: &template.map,
					level: template.map.z_level(template_z),
					min: (0, 0),
					max: (template_x - 1, template_y - 1),
					render_passes,
					errors: &errors,
					bump,
					print_errors: false,
				};
				minimap::generate(minimap_context, &dm_context.icon_cache)
					.map_err(|_| eyre!("failed to generate minimap"))?
			};
			bump.reset();
			composite_template(
				&mut image,
				template,
				template_z,
				&template_image,
				map.dim_xyz(),
			);
		}

		generate_minimap_image(
			crate::render::GeneratedMinimap {
				map_dir: map_dir.to_owned(),
				name: map_config.name.clone(),
				z: z + 1,
				image,
			},
			config,
			optimize_options,
			encode_bar,
		)
	})
}

fn composite_template(
	base_image: &mut dmm_tools::dmi::Image,
	template: &AutomapperTemplate,
	template_z: usize,
	template_image: &dmm_tools::dmi::Image,
	base_dimensions: (usize, usize, usize),
) {
	const TILE_SIZE: u32 = 32;
	let (base_x, base_y, _base_z) = base_dimensions;
	let (_template_x, template_y, _template_z) = template.map.dim_xyz();
	let dest_x = (template.x as u32 - 1) * TILE_SIZE;
	let dest_y = (base_y as u32 - (template.y + template_y - 1) as u32) * TILE_SIZE;
	let image_width = base_image.width as usize;
	let pixels = base_image
		.data
		.as_slice_mut()
		.expect("image data is not contiguous");

	for (coord, key) in template.map.z_level(template_z).iter_top_down() {
		let prefabs = &template.map.dictionary[&key];
		if prefabs
			.iter()
			.any(|prefab| prefab.path == "/turf/template_noop")
		{
			continue;
		}
		let tile_x = dest_x + (coord.x as u32 - 1) * TILE_SIZE;
		let tile_y = dest_y + (template_y as u32 - coord.y as u32) * TILE_SIZE;
		for y in tile_y..tile_y + TILE_SIZE {
			let row_start = y as usize * image_width + tile_x as usize;
			pixels[row_start..row_start + TILE_SIZE as usize].fill(Rgba8::default());
		}
	}

	debug_assert_eq!(base_image.width, base_x as u32 * TILE_SIZE);
	base_image.composite(
		template_image,
		(dest_x, dest_y),
		(0, 0, template_image.width, template_image.height),
		[u8::MAX; 4],
	);
}
