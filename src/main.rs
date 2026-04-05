// SPDX-License-Identifier: MPL-2.0
extern crate dreammaker as dm;

pub mod config;
pub mod context;

use crate::{
	config::{MapConfig, ServerConfig},
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
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use std::{
	cell::RefCell,
	sync::{Mutex, RwLock},
	time::Instant,
};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

struct RenderPassHolder {
	main: Vec<Box<dyn RenderPass>>,
	pipes: Vec<Box<dyn RenderPass>>,
}

fn create_render_passes(config: &ServerConfig, map_renderer: &MapRenderer) -> RenderPassHolder {
	let main = dmm_tools::render_passes::configure_list(
		map_renderer,
		&config.render_passes.include,
		&config.render_passes.exclude,
	);
	let pipes =
		dmm_tools::render_passes::configure_list(map_renderer, &["only-wires-and-pipes"], &[]);
	RenderPassHolder { main, pipes }
}

fn main() -> Result<()> {
	color_eyre::install()?;

	let config = {
		let config_file = std::fs::read("config.json").wrap_err("failed to read config.json")?;
		serde_json::from_slice::<ServerConfig>(&config_file)
			.wrap_err("failed to parse config.json")?
	};

	if !config.out_path.exists() {
		std::fs::create_dir_all(&config.out_path).wrap_err_with(|| {
			format!(
				"failed to create output folder at {}",
				config.out_path.display()
			)
		})?;
	}

	let mut context = dm::Context::default();
	let mut dm_context = DmContext::default();
	dm_context
		.objtree(&mut context, &config)
		.wrap_err("failed to setup obj tree")?;

	let render_passes = create_render_passes(&config, &context.config().map_renderer);

	let minimaps = Mutex::new(Vec::<GeneratedMinimap>::new());
	config.maps.par_iter().for_each(|map_config| {
		if let Err(err) =
			generate_minimap(&config, map_config, &dm_context, &render_passes, &minimaps)
		{
			thread_safe_print_err(format!(
				"failed to generate minimap for {}: {err}",
				&map_config.map_name
			));
		}
	});

	let minimaps = std::mem::take(&mut *minimaps.lock().unwrap());

	println!("optimizing {} minimaps", minimaps.len());
	let optimize_options = config.optimize_options();
	minimaps.into_par_iter().for_each(|minimap| {
		if let Err(err) = generate_minimap_image(minimap, &config, &optimize_options) {
			thread_safe_print_err(format!("failed to write minimap: {err}"));
		}
	});
	thread_safe_print("done :)");

	Ok(())
}

fn generate_minimap(
	server_config: &ServerConfig,
	map_config: &MapConfig,
	dm_context: &DmContext,
	render_passes: &RenderPassHolder,
	minimaps: &Mutex<Vec<GeneratedMinimap>>,
) -> Result<()> {
	let map_path = server_config.base_map_path().join(&map_config.dmm_path);
	let map = dmm::Map::from_file(&map_path).wrap_err_with(|| {
		format!(
			"failed to load {} from {}",
			&map_config.map_name,
			map_path.display()
		)
	})?;
	let (dim_x, dim_y, dim_z) = map.dim_xyz();
	println!(
		"{}: dim_x={dim_x}, dim_y={dim_y}, dim_z={dim_z}",
		&map_config.map_name
	);
	(0..dim_z).into_par_iter().for_each(|z| {
		if let Err(err) = generate_for_z(
			&map,
			z,
			server_config,
			map_config,
			dm_context,
			render_passes,
			minimaps,
		) {
			thread_safe_print_err(format!(
				"failed to generate minimap for {} (z={z}): {err}",
				&map_config.map_name
			));
		}
	});
	Ok(())
}

fn generate_minimap_image(
	minimap: GeneratedMinimap,
	config: &ServerConfig,
	optimize_options: &oxipng::Options,
) -> Result<()> {
	let GeneratedMinimap { name, z, image } = minimap;
	let mut start = Instant::now();
	if config.generate_webp {
		// funny thing, i find that lossless produces much smaller falls than lossy,
		// even at the same quality setting
		let raw: &[u8] = bytemuck::cast_slice(image.data.as_slice().unwrap());
		let webp = webp::Encoder::from_rgba(raw, image.width, image.height)
			.encode_lossless()
			.to_vec();
		std::fs::write(config.out_path.join(format!("{name}-{z}.webp")), webp)
			.wrap_err("failed to write webp")?;
		thread_safe_print(format!(
			"{name}-{z} webp done in {:.2} seconds",
			start.elapsed().as_secs_f64()
		));
		start = Instant::now();
	}

	let png = oxipng::RawImage::new(
		image.width,
		image.height,
		oxipng::ColorType::RGBA,
		oxipng::BitDepth::Eight,
		bytemuck::cast_vec(image.data.into_raw_vec()),
	)
	.wrap_err("failed to create raw png image")?;
	let optimized_png = png
		.create_optimized_png(optimize_options)
		.wrap_err("failed to optimize png image")?;
	std::fs::write(
		config.out_path.join(format!("{name}-{z}.png")),
		optimized_png,
	)
	.wrap_err("failed to write optimized png")?;
	thread_safe_print(format!(
		"{name}-{z} png done in {:.2} seconds",
		start.elapsed().as_secs_f64()
	));
	Ok(())
}

fn generate_for_z(
	map: &Map,
	z: usize,
	_server_config: &ServerConfig,
	map_config: &MapConfig,
	dm_context: &DmContext,
	render_passes: &RenderPassHolder,
	minimaps: &Mutex<Vec<GeneratedMinimap>>,
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
			render_passes: &render_passes.main,
			errors: &errors,
			bump,
		};
		let map_name = &map_config.map_name;
		thread_safe_print(format!("generating minimap for {map_name} (z={z})"));
		let image = minimap::generate(minimap_context, &dm_context.icon_cache)
			.map_err(|_| eyre!("failed to generate minimap"))?;
		minimaps.lock().unwrap().push(GeneratedMinimap {
			name: map_name.clone(),
			z,
			image,
		});
		Ok(())
	})
}

thread_local! {
	static BUMP: RefCell<Bump> = RefCell::new(Bump::new());
}

struct GeneratedMinimap {
	name: String,
	z: usize,
	image: dmm_tools::dmi::Image,
}

fn thread_safe_print(meow: impl AsRef<str>) {
	use std::io::Write;

	let mut stdout = std::io::stdout().lock();
	let _ = writeln!(stdout, "{}", meow.as_ref());
}

fn thread_safe_print_err(meow: impl AsRef<str>) {
	use std::io::Write;

	let mut stderr = std::io::stderr().lock();
	let _ = writeln!(stderr, "{}", meow.as_ref());
}
