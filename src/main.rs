// SPDX-License-Identifier: MPL-2.0
extern crate dreammaker as dm;

pub mod config;
pub mod context;
pub mod encode;
pub mod render;

use crate::{
	config::{MapCategory, MapConfig, ResolvedFlags, ServerConfig},
	context::DmContext,
	encode::generate_minimap_image,
	render::{
		GeneratedMinimap, MapOutputSpec, ProgressState, create_render_passes, generate_minimap,
	},
};
use color_eyre::eyre::{Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use std::{
	path::{Path, PathBuf},
	sync::Mutex,
	time::{Duration, Instant},
};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn total_bar_style() -> ProgressStyle {
	ProgressStyle::with_template(
		"{spinner:.green} [{elapsed_precise}] {bar:40.green/white.dim} {pos}/{len} {msg}",
	)
	.expect("invalid progress style template")
	.progress_chars("=>-")
	.tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", ""])
}

fn collect_category_maps<'a>(
	cat: &'a MapCategory,
	out_path: &Path,
) -> impl Iterator<Item = (&'a MapConfig, MapOutputSpec)> {
	let base_map_path = cat.base_map_path();
	let cat_name = cat.name.clone();
	let out_path = out_path.to_owned();
	let direct = cat.maps.iter().map({
		let base_map_path = base_map_path.clone();
		let cat_name = cat_name.clone();
		let out_path = out_path.clone();
		move |m| {
			let flags = ResolvedFlags::resolve(m, None, cat);
			let map_dir = out_path.join(&cat_name).join(&m.name);
			let pipes_dir = flags
				.supports_pipes
				.then(|| out_path.join(&cat_name).join("pipes").join(&m.name));
			(m, MapOutputSpec {
				base_map_path: base_map_path.clone(),
				map_dir,
				pipes_dir,
				flags,
			})
		}
	});
	let from_subs = cat.subcategories.iter().flat_map({
		let base_map_path = base_map_path.clone();
		let cat_name = cat_name.clone();
		let out_path = out_path.clone();
		move |sub| {
			let base_map_path = base_map_path.clone();
			let cat_name = cat_name.clone();
			let out_path = out_path.clone();
			let sub_name = sub.name.clone();
			sub.maps.iter().map(move |m| {
				let flags = ResolvedFlags::resolve(m, Some(sub), cat);
				let map_dir = out_path.join(&cat_name).join(&sub_name).join(&m.name);
				let pipes_dir = flags.supports_pipes.then(|| {
					out_path
						.join(&cat_name)
						.join(&sub_name)
						.join("pipes")
						.join(&m.name)
				});
				(m, MapOutputSpec {
					base_map_path: base_map_path.clone(),
					map_dir,
					pipes_dir,
					flags,
				})
			})
		}
	});
	direct.chain(from_subs)
}

fn main() -> Result<()> {
	color_eyre::install()?;

	let t_config = Instant::now();
	let config = {
		let config_file = std::fs::read("config.json").wrap_err("failed to read config.json")?;
		serde_json::from_slice::<ServerConfig>(&config_file)
			.wrap_err("failed to parse config.json")?
	};
	let elapsed_config = t_config.elapsed();

	if !config.out_path.exists() {
		std::fs::create_dir_all(&config.out_path).wrap_err_with(|| {
			format!(
				"failed to create output folder at {}",
				config.out_path.display()
			)
		})?;
	}

	let progress = {
		let mp = MultiProgress::new();
		let total = mp.add(ProgressBar::new(0));
		total.set_style(total_bar_style());
		total.set_message("rendering");
		total.enable_steady_tick(Duration::from_millis(100));
		ProgressState { mp, total }
	};

	// Collect unique (game_path, env_file) pairs in order to avoid redundant env
	// parses
	let mut env_keys: Vec<(PathBuf, String)> = Vec::new();
	for cat in &config.categories {
		let key = (cat.game_path.clone(), cat.env_file.clone());
		if !env_keys.contains(&key) {
			env_keys.push(key);
		}
	}

	let minimaps = Mutex::new(Vec::<GeneratedMinimap>::new());

	let mut elapsed_parse = Duration::ZERO;
	let mut elapsed_render = Duration::ZERO;

	for (game_path, env_file) in &env_keys {
		let t_parse = Instant::now();
		let mut dm_ctx = dm::Context::default();
		let mut dm_context = DmContext::default();
		dm_context
			.objtree(&mut dm_ctx, game_path, env_file)
			.wrap_err("failed to setup obj tree")?;
		elapsed_parse += t_parse.elapsed();

		let render_passes = create_render_passes(&config, &dm_ctx.config().map_renderer);

		let group_maps: Vec<_> = config
			.categories
			.iter()
			.filter(|cat| &cat.game_path == game_path && &cat.env_file == env_file)
			.flat_map(|cat| collect_category_maps(cat, &config.out_path))
			.collect();

		let t_render = Instant::now();
		group_maps.par_iter().for_each(|(map_config, output)| {
			if let Err(err) = generate_minimap(
				map_config,
				&dm_context,
				&render_passes,
				&minimaps,
				output.clone(),
				&progress,
			) {
				progress.total.println(format!(
					"failed to generate minimap for {}: {err}",
					&map_config.name
				));
			}
		});
		elapsed_render += t_render.elapsed();
	}

	let minimaps = std::mem::take(&mut *minimaps.lock().unwrap());

	progress.total.set_message("encoding");
	progress.total.inc_length(minimaps.len() as u64);
	let optimize_options = config.optimize_options();
	let t_encode = Instant::now();
	minimaps.into_par_iter().for_each(|minimap| {
		if let Err(err) =
			generate_minimap_image(minimap, &config, &optimize_options, &progress.total)
		{
			progress
				.total
				.println(format!("failed to write minimap: {err}"));
		}
	});
	let elapsed_encode = t_encode.elapsed();

	progress.total.finish_and_clear();
	progress.mp.clear().ok();

	println!("done :)");
	println!("  config load:  {:.2?}", elapsed_config);
	println!("  env parse:    {:.2?}", elapsed_parse);
	println!("  render:       {:.2?}", elapsed_render);
	println!("  encode:       {:.2?}", elapsed_encode);

	Ok(())
}
