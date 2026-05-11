// SPDX-License-Identifier: MPL-2.0
extern crate dreammaker as dm;

pub mod config;
pub mod context;
pub mod encode;
pub mod render;
pub mod util;

use crate::{
	config::ServerConfig,
	context::DmContext,
	encode::generate_minimap_image,
	render::{GeneratedMinimap, create_render_passes, generate_minimap},
	util::{thread_safe_print, thread_safe_print_err},
};
use color_eyre::eyre::{Context, Result};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use std::sync::Mutex;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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

	thread_safe_print(format!("optimizing {} minimaps", minimaps.len()));
	let optimize_options = config.optimize_options();
	minimaps.into_par_iter().for_each(|minimap| {
		if let Err(err) = generate_minimap_image(minimap, &config, &optimize_options) {
			thread_safe_print_err(format!("failed to write minimap: {err}"));
		}
	});
	thread_safe_print("done :)");

	Ok(())
}
