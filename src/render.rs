// SPDX-License-Identifier: MPL-2.0
use crate::{
	config::{MapConfig, ResolvedFlags, ServerConfig},
	context::DmContext,
	util::{thread_safe_print, thread_safe_print_err},
};
use bumpalo::Bump;
use color_eyre::eyre::{Context, Result, eyre};
use dm::config::MapRenderer;
use dmm_tools::{
	dmm::{self, Map},
	minimap,
	render_passes::RenderPass,
};
use std::{
	cell::RefCell,
	path::{Path, PathBuf},
	sync::{Mutex, RwLock},
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
	pub map_dir: PathBuf,
	pub pipes_dir: Option<PathBuf>,
	pub flags: ResolvedFlags,
}

thread_local! {
	static BUMP: RefCell<Bump> = RefCell::new(Bump::new());
}

pub fn generate_minimap(
	server_config: &ServerConfig,
	map_config: &MapConfig,
	dm_context: &DmContext,
	render_passes: &RenderPassHolder,
	minimaps: &Mutex<Vec<GeneratedMinimap>>,
	output: MapOutputSpec,
) -> Result<()> {
	let MapOutputSpec {
		map_dir,
		pipes_dir,
		flags,
	} = output;
	use rayon::iter::{IntoParallelIterator, ParallelIterator};

	let map_path = server_config.base_map_path().join(&map_config.dmm_path);
	let map = dmm::Map::from_file(&map_path).wrap_err_with(|| {
		format!(
			"failed to load {} from {}",
			&map_config.map_name,
			map_path.display()
		)
	})?;
	let (dim_x, dim_y, dim_z) = map.dim_xyz();
	thread_safe_print(format!(
		"{}: dim_x={dim_x}, dim_y={dim_y}, dim_z={dim_z}",
		&map_config.map_name
	));

	if flags.render_once {
		let all_exist = (1..=dim_z).all(|z| {
			map_dir
				.join(format!("{}-{z}.png", &map_config.map_name))
				.exists()
		});
		if all_exist {
			thread_safe_print(format!(
				"{}: skipping (renderOnce, outputs exist)",
				&map_config.map_name
			));
			return Ok(());
		}
	}

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
			thread_safe_print_err(format!(
				"failed to generate minimap for {} (z={z}): {err}",
				&map_config.map_name
			));
		}
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
				thread_safe_print_err(format!(
					"failed to generate pipes minimap for {} (z={z}): {err}",
					&map_config.map_name
				));
			}
		});
	}

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
	BUMP.with_borrow(|bump| {
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
		};
		let map_name = &map_config.map_name;
		thread_safe_print(format!("generating minimap for {map_name} (z={})", z + 1));
		let image = minimap::generate(minimap_context, &dm_context.icon_cache)
			.map_err(|_| eyre!("failed to generate minimap"))?;
		minimaps.lock().unwrap().push(GeneratedMinimap {
			map_dir: map_dir.to_owned(),
			name: map_name.to_string(),
			z: z + 1,
			image,
		});
		Ok(())
	})
}
