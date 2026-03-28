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
use dmm_tools::{
	dmm::{self, Map},
	minimap,
	render_passes::RenderPass,
};
use std::{cell::RefCell, sync::RwLock};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> Result<()> {
	color_eyre::install()?;

	let config = {
		let config_file = std::fs::read("config.json").wrap_err("failed to read config.json")?;
		serde_json::from_slice::<ServerConfig>(&config_file)
			.wrap_err("failed to parse config.json")?
	};

	if !config.out_folder.exists() {
		std::fs::create_dir_all(&config.out_folder).wrap_err_with(|| {
			format!(
				"failed to create output folder at {}",
				config.out_folder.display()
			)
		})?;
	}

	let mut dm_context = DmContext::default();
	dm_context
		.objtree(&config)
		.wrap_err("failed to setup obj tree")?;

	let render_passes = dmm_tools::render_passes::configure_list(
		&dm_context.dm_context.config().map_renderer,
		&config.render_passes.include,
		&config.render_passes.exclude,
	);

	let base_map_path = config.game_path.join(&config.map_files_path);
	let mut minimaps = Vec::<GeneratedMinimap>::new();
	for map_config in &config.maps {
		let map_path = base_map_path.join(&map_config.dmm_path);
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
		for z in 0..dim_z {
			generate_for_z(
				&map,
				z,
				&config,
				map_config,
				&dm_context,
				&render_passes,
				&mut minimaps,
			)
			.wrap_err_with(|| {
				format!(
					"failed to generate minimap for {} (z={z})",
					&map_config.map_name
				)
			})?;
		}
	}

	println!("optimizing {} minimaps", minimaps.len());
	let optimize_options = config.optimize_options();
	for GeneratedMinimap { name, z, image } in minimaps {
		if config.generate_webp {
			// funny thing, i find that lossless produces much smaller falls than lossy,
			// even at the same quality setting
			let raw: &[u8] = bytemuck::cast_slice(image.data.as_slice().unwrap());
			let webp = webp::Encoder::from_rgba(raw, image.width, image.height)
				.encode_lossless()
				.to_vec();
			std::fs::write(config.out_folder.join(format!("{name}-{z}.webp")), webp)
				.wrap_err("failed to write webp")?;
			println!("{name}-{z} webp done");
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
			.create_optimized_png(&optimize_options)
			.wrap_err("failed to optimize png image")?;
		std::fs::write(
			config.out_folder.join(format!("{name}-{z}.png")),
			optimized_png,
		)
		.wrap_err("failed to write optimized png")?;
		println!("{name}-{z} png done");
	}
	println!("done :)");

	Ok(())
}

fn generate_for_z(
	map: &Map,
	z: usize,
	_server_config: &ServerConfig,
	map_config: &MapConfig,
	dm_context: &DmContext,
	render_passes: &[Box<dyn RenderPass>],
	minimaps: &mut Vec<GeneratedMinimap>,
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
		println!("generating minimap for {map_name} (z={z})");
		let image = minimap::generate(minimap_context, &dm_context.icon_cache)
			.map_err(|_| eyre!("failed to generate minimap"))?;
		minimaps.push(GeneratedMinimap {
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
