// SPDX-License-Identifier: MPL-2.0
use crate::{
	config::{MapConfig, ResolvedFlags, ServerConfig},
	context::DmContext,
};
use bumpalo::Bump;
use color_eyre::eyre::{Context, Result, eyre};
use dm::config::MapRenderer;
use dmm_tools::{
	dmm::{self, Map},
	minimap,
	render_passes::RenderPass,
};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::{
	cell::RefCell,
	path::{Path, PathBuf},
	sync::{Mutex, RwLock},
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
	minimaps: &Mutex<Vec<GeneratedMinimap>>,
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
	total_bar.println(format!(
		"{}: dim_x={dim_x}, dim_y={dim_y}, dim_z={dim_z}",
		map_config.name
	));

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
			minimaps,
			&map_dir,
		) {
			total_bar.println(format!(
				"failed to generate minimap for {} (z={}): {err}",
				map_config.name,
				z + 1
			));
		}
		map_bar.inc(1);
		total_bar.inc(1);
	});

	if let Some(ref pipes_dir) = pipes_dir {
		(0..dim_z).into_par_iter().for_each(|z| {
			if let Err(err) = generate_for_z(
				&map,
				z,
				map_config,
				dm_context,
				&render_passes.pipes,
				minimaps,
				pipes_dir,
			) {
				total_bar.println(format!(
					"failed to generate pipes minimap for {} (z={}): {err}",
					map_config.name,
					z + 1
				));
			}
			map_bar.inc(1);
			total_bar.inc(1);
		});
	}

	map_bar.finish_and_clear();
	Ok(())
}

fn generate_for_z(
	map: &Map,
	z: usize,
	map_config: &MapConfig,
	dm_context: &DmContext,
	render_passes: &[Box<dyn RenderPass>],
	minimaps: &Mutex<Vec<GeneratedMinimap>>,
	map_dir: &Path,
) -> Result<()> {
	let errors = RwLock::default();
	BUMP.with_borrow_mut(|bump| {
		let (dim_x, dim_y, _dim_z) = map.dim_xyz();
		let map_name = &map_config.name;
		let image = {
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
				.map_err(|_| eyre!("failed to generate minimap"))
		};
		bump.reset();
		let image = image?; // just ensures the bump allocator is reset even if it errors. kinda stupid but whatever idc
		minimaps.lock().unwrap().push(GeneratedMinimap {
			map_dir: map_dir.to_owned(),
			name: map_name.to_string(),
			z: z + 1,
			image,
		});
		Ok(())
	})
}
